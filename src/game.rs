//! Mesin permainan: lobby → baca soal → jawab → hasil → papan skor → akhir.

use crate::json::Json;
use crate::obj;
use crate::quiz::Question;
use crate::util::{rand_hex, Rng};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

pub enum Out {
    Text(String),
    Pong(Vec<u8>),
    Close,
}
pub type Tx = Sender<Out>;

fn send(tx: &Tx, v: &Json) {
    let _ = tx.send(Out::Text(v.dump()));
}

// ───────────────────────── Penjadwal timer ─────────────────────────

type Job = (Instant, Arc<Game>, u64);

fn queue() -> &'static Mutex<Vec<Job>> {
    static Q: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn start_scheduler() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_millis(40));
        let due: Vec<Job> = {
            let mut q = queue().lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let all: Vec<Job> = q.drain(..).collect();
            let (due, rest): (Vec<Job>, Vec<Job>) = all.into_iter().partition(|j| j.0 <= now);
            *q = rest;
            due
        };
        for (_, g, epoch) in due {
            g.on_timer(epoch);
        }
    });
}

// ───────────────────────── Pengaturan ─────────────────────────

#[derive(Clone, Debug)]
pub struct Settings {
    pub time_override: Option<u32>,
    pub read_secs: u32,
    pub points_mode: String, // standard | double | flat
    pub streak_bonus: bool,
    pub shuffle_questions: bool,
    pub shuffle_answers: bool,
    pub show_board: bool,
    pub auto_next_secs: u32,
    pub allow_late_join: bool,
    pub show_text_on_player: bool,
    pub max_players: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            time_override: None,
            read_secs: 3,
            points_mode: "standard".into(),
            streak_bonus: true,
            shuffle_questions: false,
            shuffle_answers: false,
            show_board: true,
            auto_next_secs: 0,
            allow_late_join: true,
            show_text_on_player: true,
            max_players: 300,
        }
    }
}

impl Settings {
    pub fn from_json(j: &Json) -> Settings {
        let mut s = Settings::default();
        if let Some(v) = j.get("time_override") {
            s.time_override = v.as_u32().filter(|t| *t > 0);
        }
        if let Some(v) = j.get("read_secs").and_then(|v| v.as_u32()) { s.read_secs = v.min(15); }
        if let Some(v) = j.get("points_mode").and_then(|v| v.as_str()) {
            if matches!(v, "standard" | "double" | "flat") {
                s.points_mode = v.to_string();
            }
        }
        if let Some(v) = j.get("streak_bonus").and_then(|v| v.as_bool()) { s.streak_bonus = v; }
        if let Some(v) = j.get("shuffle_questions").and_then(|v| v.as_bool()) { s.shuffle_questions = v; }
        if let Some(v) = j.get("shuffle_answers").and_then(|v| v.as_bool()) { s.shuffle_answers = v; }
        if let Some(v) = j.get("show_board").and_then(|v| v.as_bool()) { s.show_board = v; }
        if let Some(v) = j.get("auto_next_secs").and_then(|v| v.as_u32()) { s.auto_next_secs = v.min(60); }
        if let Some(v) = j.get("allow_late_join").and_then(|v| v.as_bool()) { s.allow_late_join = v; }
        if let Some(v) = j.get("show_text_on_player").and_then(|v| v.as_bool()) { s.show_text_on_player = v; }
        if let Some(v) = j.get("max_players").and_then(|v| v.as_u32()) { s.max_players = v.clamp(1, 1000); }
        if let Some(t) = s.time_override {
            s.time_override = Some(t.clamp(1, 600));
        }
        s
    }

    pub fn to_json(&self) -> Json {
        obj! {
            "time_override" => self.time_override,
            "read_secs" => self.read_secs,
            "points_mode" => &self.points_mode,
            "streak_bonus" => self.streak_bonus,
            "shuffle_questions" => self.shuffle_questions,
            "shuffle_answers" => self.shuffle_answers,
            "show_board" => self.show_board,
            "auto_next_secs" => self.auto_next_secs,
            "allow_late_join" => self.allow_late_join,
            "show_text_on_player" => self.show_text_on_player,
            "max_players" => self.max_players,
        }
    }
}

// ───────────────────────── Data ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Lobby,
    Read,
    Question,
    Reveal,
    Board,
    Final,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Lobby => "lobby",
            Phase::Read => "read",
            Phase::Question => "question",
            Phase::Reveal => "reveal",
            Phase::Board => "board",
            Phase::Final => "final",
        }
    }
}

#[derive(Clone)]
pub struct Ans {
    pub choices: Vec<usize>,
    pub ok: bool,
    pub points: u32,
    pub bonus: u32,
    pub ms: u32,
}

impl Ans {
    fn to_json(&self) -> Json {
        obj! {"choices" => &self.choices, "ok" => self.ok, "points" => self.points, "bonus" => self.bonus, "ms" => self.ms}
    }
}

struct Player {
    id: String,
    name: String,
    score: u32,
    streak: u32,
    answers: Vec<Option<Ans>>,
    tx: Option<Tx>,
    conn: u64,
}

pub struct QStat {
    pub index: usize,
    pub text: String,
    pub answers: Vec<String>,
    pub correct: Vec<usize>,
    pub counts: Vec<u32>,
    pub answered: u32,
    pub correct_n: u32,
    pub avg_ms: u32,
    pub time: u32,
}

pub struct PStat {
    pub rank: usize,
    pub name: String,
    pub score: u32,
    pub correct_n: u32,
    pub wrong_n: u32,
    pub skipped_n: u32,
    pub answers: Vec<Option<Ans>>,
}

pub struct Report {
    pub title: String,
    pub total_players: usize,
    pub questions: Vec<QStat>,
    pub players: Vec<PStat>,
}

impl Report {
    pub fn to_json(&self) -> Json {
        obj! {
            "title" => &self.title,
            "total_players" => self.total_players,
            "questions" => self.questions.iter().map(|q| obj!{
                "index" => q.index, "text" => &q.text, "answers" => &q.answers, "correct" => &q.correct,
                "counts" => &q.counts, "answered" => q.answered, "correct_n" => q.correct_n,
                "avg_ms" => q.avg_ms, "time" => q.time,
            }).collect::<Vec<Json>>(),
            "players" => self.players.iter().map(|p| obj!{
                "rank" => p.rank, "name" => &p.name, "score" => p.score,
                "correct_n" => p.correct_n, "wrong_n" => p.wrong_n, "skipped_n" => p.skipped_n,
                "answers" => p.answers.iter().map(|a| match a { Some(a) => a.to_json(), None => Json::Null }).collect::<Vec<Json>>(),
            }).collect::<Vec<Json>>(),
        }
    }
}

struct Inner {
    pin: String,
    title: String,
    base: Vec<Question>,
    questions: Vec<Question>,
    settings: Settings,
    phase: Phase,
    epoch: u64,
    qi: usize,
    phase_started: Instant,
    deadline: Option<Instant>,
    q_open: Instant,
    q_time_ms: u32,
    players: Vec<Player>,
    hosts: Vec<(u64, Tx)>,
    last_activity: Instant,
    locked: bool,
}

pub enum HostCmd {
    Start,
    Next,
    Skip,
    End,
    Reset,
    AddTime(u32),
    Kick(String),
    Lock(bool),
}

pub struct Game {
    pub pin: String,
    pub host_token: String,
    inner: Mutex<Inner>,
}

fn prepare(base: &[Question], s: &Settings) -> Vec<Question> {
    let mut rng = Rng::new();
    let mut qs: Vec<Question> = base.to_vec();
    if s.shuffle_questions {
        rng.shuffle(&mut qs);
    }
    for q in qs.iter_mut() {
        if let Some(t) = s.time_override {
            q.time = t;
        }
        if s.shuffle_answers {
            let n = q.answers.len();
            let mut perm: Vec<usize> = (0..n).collect();
            rng.shuffle(&mut perm);
            let old = q.clone();
            q.answers = perm.iter().map(|&o| old.answers[o].clone()).collect();
            q.correct = (0..n).filter(|j| old.correct.contains(&perm[*j])).collect();
        }
    }
    qs
}

fn clean_name(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().filter(|c| !c.is_control()).take(20).collect()
}

impl Inner {
    fn n(&self) -> usize {
        self.questions.len()
    }

    fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    fn ms_left(&self) -> u64 {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()).as_millis() as u64)
            .unwrap_or(0)
    }

    fn auto_left(&self) -> Json {
        if self.settings.auto_next_secs == 0 {
            return Json::Null;
        }
        let total = self.settings.auto_next_secs as u64 * 1000;
        let el = self.phase_started.elapsed().as_millis() as u64;
        Json::from(total.saturating_sub(el))
    }

    fn order(&self) -> Vec<usize> {
        let mut o: Vec<usize> = (0..self.players.len()).collect();
        o.sort_by(|&a, &b| self.players[b].score.cmp(&self.players[a].score).then(a.cmp(&b)));
        o
    }

    fn rank_of(order: &[usize], i: usize) -> usize {
        order.iter().position(|&x| x == i).map(|p| p + 1).unwrap_or(0)
    }

    fn q_json(&self, qi: usize, with_answers: bool, with_correct: bool, hide_text: bool) -> Json {
        let q = &self.questions[qi];
        let mut v = obj! {
            "index" => qi,
            "total" => self.n(),
            "text" => if hide_text { "" } else { q.text.as_str() },
            "time" => q.time,
            "multi" => q.correct.len() > 1,
            "n_answers" => q.answers.len(),
        };
        if with_answers {
            let ans: Vec<String> = if hide_text {
                q.answers.iter().map(|_| String::new()).collect()
            } else {
                q.answers.clone()
            };
            v.set("answers", ans);
        }
        if with_correct {
            v.set("correct", &q.correct);
        }
        v
    }

    fn counts(&self, qi: usize) -> Vec<u32> {
        let mut c = vec![0u32; self.questions[qi].answers.len()];
        for p in &self.players {
            if let Some(a) = p.answers.get(qi).and_then(|a| a.as_ref()) {
                for &ch in &a.choices {
                    if ch < c.len() {
                        c[ch] += 1;
                    }
                }
            }
        }
        c
    }

    fn answered_count(&self, qi: usize) -> usize {
        self.players.iter().filter(|p| p.answers.get(qi).map(|a| a.is_some()).unwrap_or(false)).count()
    }

    fn online_count(&self) -> usize {
        self.players.iter().filter(|p| p.tx.is_some()).count()
    }

    fn all_answered(&self) -> bool {
        let mut online = self.players.iter().filter(|p| p.tx.is_some()).peekable();
        if online.peek().is_none() {
            return false;
        }
        online.all(|p| p.answers[self.qi].is_some())
    }

    fn top_json(&self, order: &[usize], n: usize) -> Json {
        let items: Vec<Json> = order
            .iter()
            .take(n)
            .enumerate()
            .map(|(r, &i)| {
                let p = &self.players[i];
                let gain = p.answers.get(self.qi).and_then(|a| a.as_ref()).map(|a| a.points).unwrap_or(0);
                obj! {"rank" => r + 1, "name" => &p.name, "score" => p.score, "gain" => gain, "streak" => p.streak}
            })
            .collect();
        Json::Arr(items)
    }

    fn behind(&self, order: &[usize], rank: usize, my_score: u32) -> Json {
        if rank > 1 {
            let j = order[rank - 2];
            obj! {"name" => &self.players[j].name, "diff" => self.players[j].score.saturating_sub(my_score)}
        } else {
            Json::Null
        }
    }

    fn host_state(&self) -> Json {
        let mut v = obj! {
            "t" => "state", "role" => "host", "pin" => &self.pin, "title" => &self.title,
            "phase" => self.phase.as_str(), "qi" => self.qi, "total" => self.n(),
            "players_total" => self.players.len(), "online" => self.online_count(), "locked" => self.locked,
        };
        match self.phase {
            Phase::Lobby => {
                let ps: Vec<Json> = self
                    .players
                    .iter()
                    .map(|p| obj! {"id" => &p.id, "name" => &p.name, "online" => p.tx.is_some()})
                    .collect();
                v.set("players", ps);
                v.set("settings", self.settings.to_json());
            }
            Phase::Read => {
                v.set("q", self.q_json(self.qi, false, false, false));
                v.set("ms_left", self.ms_left());
            }
            Phase::Question => {
                v.set("q", self.q_json(self.qi, true, false, false));
                v.set("ms_left", self.ms_left());
                v.set("answered", self.answered_count(self.qi));
            }
            Phase::Reveal => {
                v.set("q", self.q_json(self.qi, true, true, false));
                v.set("counts", self.counts(self.qi));
                let ans = self.answered_count(self.qi);
                let cor = self
                    .players
                    .iter()
                    .filter(|p| p.answers[self.qi].as_ref().map(|a| a.ok).unwrap_or(false))
                    .count();
                v.set("answered", ans);
                v.set("correct_n", cor);
                v.set("auto_ms", self.auto_left());
            }
            Phase::Board => {
                let order = self.order();
                v.set("top", self.top_json(&order, 5));
                v.set("last", self.qi + 1 >= self.n());
                v.set("auto_ms", self.auto_left());
            }
            Phase::Final => {
                v.set("report", self.report().to_json());
            }
        }
        v
    }

    fn player_state(&self, i: usize, order: &[usize]) -> Json {
        let p = &self.players[i];
        let rank = Self::rank_of(order, i);
        let hide = !self.settings.show_text_on_player;
        let mut v = obj! {
            "t" => "state", "role" => "player", "phase" => self.phase.as_str(),
            "name" => &p.name, "score" => p.score, "rank" => rank, "streak" => p.streak,
            "players_total" => self.players.len(), "qi" => self.qi, "total" => self.n(),
        };
        match self.phase {
            Phase::Lobby => {}
            Phase::Read => {
                v.set("q", self.q_json(self.qi, false, false, hide));
                v.set("ms_left", self.ms_left());
            }
            Phase::Question => {
                v.set("q", self.q_json(self.qi, true, false, hide));
                v.set("ms_left", self.ms_left());
                let mine = p.answers[self.qi].as_ref();
                v.set("answered", mine.is_some());
                v.set("mine", mine.map(|a| a.choices.clone()).unwrap_or_default());
            }
            Phase::Reveal => {
                v.set("q", self.q_json(self.qi, true, true, hide));
                let a = p.answers[self.qi].as_ref();
                v.set(
                    "result",
                    obj! {
                        "answered" => a.is_some(),
                        "ok" => a.map(|a| a.ok).unwrap_or(false),
                        "points" => a.map(|a| a.points).unwrap_or(0),
                        "bonus" => a.map(|a| a.bonus).unwrap_or(0),
                        "mine" => a.map(|a| a.choices.clone()).unwrap_or_default(),
                        "behind" => self.behind(order, rank, p.score),
                    },
                );
                v.set("auto_ms", self.auto_left());
            }
            Phase::Board => {
                v.set("top", self.top_json(order, 5));
                v.set("behind", self.behind(order, rank, p.score));
                v.set("last", self.qi + 1 >= self.n());
            }
            Phase::Final => {
                let mut correct_n = 0u32;
                let mut wrong_n = 0u32;
                let mut skipped = 0u32;
                let review: Vec<Json> = (0..self.n())
                    .map(|qi| {
                        let q = &self.questions[qi];
                        let a = p.answers[qi].as_ref();
                        match a {
                            Some(a) if a.ok => correct_n += 1,
                            Some(_) => wrong_n += 1,
                            None => skipped += 1,
                        }
                        obj! {
                            "index" => qi, "text" => &q.text, "answers" => &q.answers, "correct" => &q.correct,
                            "mine" => a.map(|a| a.choices.clone()),
                            "ok" => a.map(|a| a.ok),
                            "points" => a.map(|a| a.points).unwrap_or(0),
                            "ms" => a.map(|a| a.ms),
                        }
                    })
                    .collect();
                v.set("review", review);
                v.set("correct_n", correct_n);
                v.set("wrong_n", wrong_n);
                v.set("skipped_n", skipped);
                v.set("top", self.top_json(order, 3));
            }
        }
        v
    }

    fn report(&self) -> Report {
        let n = self.n();
        let order = self.order();
        let questions = (0..n)
            .map(|qi| {
                let q = &self.questions[qi];
                let mut answered = 0u32;
                let mut correct_n = 0u32;
                let mut ms_sum = 0u64;
                for p in &self.players {
                    if let Some(a) = p.answers.get(qi).and_then(|a| a.as_ref()) {
                        answered += 1;
                        ms_sum += a.ms as u64;
                        if a.ok {
                            correct_n += 1;
                        }
                    }
                }
                QStat {
                    index: qi,
                    text: q.text.clone(),
                    answers: q.answers.clone(),
                    correct: q.correct.clone(),
                    counts: self.counts(qi),
                    answered,
                    correct_n,
                    avg_ms: if answered > 0 { (ms_sum / answered as u64) as u32 } else { 0 },
                    time: q.time,
                }
            })
            .collect();
        let players = order
            .iter()
            .enumerate()
            .map(|(r, &i)| {
                let p = &self.players[i];
                let correct_n = p.answers.iter().flatten().filter(|a| a.ok).count() as u32;
                let answered = p.answers.iter().flatten().count() as u32;
                PStat {
                    rank: r + 1,
                    name: p.name.clone(),
                    score: p.score,
                    correct_n,
                    wrong_n: answered - correct_n,
                    skipped_n: n as u32 - answered,
                    answers: p.answers.clone(),
                }
            })
            .collect();
        Report { title: self.title.clone(), total_players: self.players.len(), questions, players }
    }

    fn broadcast(&self) {
        let order = self.order();
        let hs = self.host_state();
        for (_, tx) in &self.hosts {
            send(tx, &hs);
        }
        for (i, p) in self.players.iter().enumerate() {
            if let Some(tx) = &p.tx {
                send(tx, &self.player_state(i, &order));
            }
        }
    }

    fn send_hosts(&self) {
        if self.hosts.is_empty() {
            return;
        }
        let hs = self.host_state();
        for (_, tx) in &self.hosts {
            send(tx, &hs);
        }
    }

    fn base_points(&self, elapsed_ms: u32) -> u32 {
        let t = self.q_time_ms.max(1) as f64;
        let e = (elapsed_ms as f64).min(t);
        match self.settings.points_mode.as_str() {
            "flat" => 1000,
            "double" => ((1.0 - (e / t) / 2.0) * 2000.0).round() as u32,
            _ => ((1.0 - (e / t) / 2.0) * 1000.0).round() as u32,
        }
    }

    // ── transisi fase ──

    fn enter_read(&mut self, g: &Arc<Game>) {
        if self.qi >= self.n() {
            return self.enter_final();
        }
        if self.settings.read_secs == 0 {
            return self.enter_question(g);
        }
        let d = Duration::from_secs(self.settings.read_secs as u64);
        self.phase = Phase::Read;
        self.phase_started = Instant::now();
        self.deadline = Some(Instant::now() + d);
        self.epoch += 1;
        g.schedule(self.epoch, d);
        self.broadcast();
    }

    fn enter_question(&mut self, g: &Arc<Game>) {
        let now = Instant::now();
        self.phase = Phase::Question;
        self.phase_started = now;
        self.q_open = now;
        self.q_time_ms = self.questions[self.qi].time * 1000;
        let d = Duration::from_millis(self.q_time_ms as u64);
        self.deadline = Some(now + d);
        self.epoch += 1;
        g.schedule(self.epoch, d);
        self.broadcast();
    }

    fn enter_reveal(&mut self, g: &Arc<Game>) {
        let qi = self.qi;
        for p in self.players.iter_mut() {
            if p.answers[qi].is_none() {
                p.streak = 0;
            }
        }
        self.phase = Phase::Reveal;
        self.phase_started = Instant::now();
        self.deadline = None;
        self.epoch += 1;
        if self.settings.auto_next_secs > 0 {
            g.schedule(self.epoch, Duration::from_secs(self.settings.auto_next_secs as u64));
        }
        self.broadcast();
    }

    fn after_reveal(&mut self, g: &Arc<Game>) {
        if self.qi + 1 >= self.n() {
            self.enter_final();
        } else if self.settings.show_board {
            self.enter_board(g);
        } else {
            self.qi += 1;
            self.enter_read(g);
        }
    }

    fn enter_board(&mut self, g: &Arc<Game>) {
        self.phase = Phase::Board;
        self.phase_started = Instant::now();
        self.deadline = None;
        self.epoch += 1;
        if self.settings.auto_next_secs > 0 {
            g.schedule(self.epoch, Duration::from_secs(self.settings.auto_next_secs as u64));
        }
        self.broadcast();
    }

    fn next_question(&mut self, g: &Arc<Game>) {
        self.qi += 1;
        self.enter_read(g);
    }

    fn enter_final(&mut self) {
        self.phase = Phase::Final;
        self.deadline = None;
        self.epoch += 1;
        self.broadcast();
    }
}

// ───────────────────────── API publik ─────────────────────────

impl Game {
    pub fn new(pin: String, title: String, questions: Vec<Question>, settings: Settings) -> Arc<Game> {
        let qs = prepare(&questions, &settings);
        let now = Instant::now();
        Arc::new(Game {
            pin: pin.clone(),
            host_token: rand_hex(12),
            inner: Mutex::new(Inner {
                pin,
                title,
                base: questions,
                questions: qs,
                settings,
                phase: Phase::Lobby,
                epoch: 0,
                qi: 0,
                phase_started: now,
                deadline: None,
                q_open: now,
                q_time_ms: 0,
                players: Vec::new(),
                hosts: Vec::new(),
                last_activity: now,
                locked: false,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn schedule(self: &Arc<Self>, epoch: u64, delay: Duration) {
        queue()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((Instant::now() + delay, Arc::clone(self), epoch));
    }

    fn on_timer(self: &Arc<Self>, epoch: u64) {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        if st.epoch != epoch {
            return;
        }
        match st.phase {
            Phase::Read => st.enter_question(self),
            Phase::Question => st.enter_reveal(self),
            Phase::Reveal => st.after_reveal(self),
            Phase::Board => st.next_question(self),
            _ => {}
        }
    }

    pub fn exists_json(&self) -> Json {
        let st = self.lock();
        obj! {"exists" => true, "phase" => st.phase.as_str(), "locked" => st.locked,
              "joinable" => !st.locked && (st.phase == Phase::Lobby || (st.settings.allow_late_join && st.phase != Phase::Final))}
    }

    pub fn is_stale(&self, max_idle: Duration) -> bool {
        let st = self.lock();
        st.hosts.is_empty() && st.players.iter().all(|p| p.tx.is_none()) && st.last_activity.elapsed() > max_idle
    }

    pub fn report(&self) -> Report {
        self.lock().report()
    }

    pub fn connect_host(&self, conn: u64, tx: Tx) {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        st.touch();
        send(&tx, &st.host_state());
        st.hosts.push((conn, tx));
    }

    pub fn disconnect(self: &Arc<Self>, conn: u64) {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        st.touch();
        st.hosts.retain(|(c, _)| *c != conn);
        let mut found = false;
        for p in st.players.iter_mut() {
            if p.conn == conn && p.tx.is_some() {
                p.tx = None;
                found = true;
            }
        }
        if found {
            if st.phase == Phase::Question && st.all_answered() {
                st.enter_reveal(self);
            } else {
                st.send_hosts();
            }
        }
    }

    pub fn join(&self, conn: u64, tx: Tx, name: &str, pid: Option<&str>) -> Result<(), String> {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        st.touch();
        // sambung ulang pemain yang sudah ada
        if let Some(pid) = pid {
            if let Some(i) = st.players.iter().position(|p| p.id == pid) {
                {
                    let p = &mut st.players[i];
                    p.tx = Some(tx.clone());
                    p.conn = conn;
                    send(&tx, &obj! {"t" => "joined", "pid" => &p.id, "name" => &p.name});
                }
                let order = st.order();
                send(&tx, &st.player_state(i, &order));
                st.send_hosts();
                return Ok(());
            }
        }
        let name = clean_name(name);
        if name.is_empty() {
            return Err("Nama tidak boleh kosong".into());
        }
        if st.phase == Phase::Final {
            return Err("Permainan ini sudah selesai".into());
        }
        if st.locked {
            return Err("Permainan dikunci oleh host".into());
        }
        if st.phase != Phase::Lobby && !st.settings.allow_late_join {
            return Err("Permainan sudah dimulai".into());
        }
        if st.players.len() >= st.settings.max_players as usize {
            return Err("Ruangan sudah penuh".into());
        }
        let lower = name.to_lowercase();
        if st.players.iter().any(|p| p.name.to_lowercase() == lower) {
            return Err("Nama itu sudah dipakai, coba nama lain".into());
        }
        let id = rand_hex(8);
        let n = st.n();
        st.players.push(Player {
            id: id.clone(),
            name: name.clone(),
            score: 0,
            streak: 0,
            answers: vec![None; n],
            tx: Some(tx.clone()),
            conn,
        });
        send(&tx, &obj! {"t" => "joined", "pid" => id, "name" => name});
        let order = st.order();
        let i = st.players.len() - 1;
        send(&tx, &st.player_state(i, &order));
        st.send_hosts();
        Ok(())
    }

    pub fn answer(self: &Arc<Self>, conn: u64, choices: Vec<usize>) {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        if st.phase != Phase::Question {
            return;
        }
        let now = Instant::now();
        if let Some(d) = st.deadline {
            if now > d + Duration::from_millis(400) {
                return;
            }
        }
        let qi = st.qi;
        let Some(pi) = st.players.iter().position(|p| p.conn == conn && p.tx.is_some()) else {
            return;
        };
        if st.players[pi].answers[qi].is_some() {
            return;
        }
        let (n_ans, multi, correct) = {
            let q = &st.questions[qi];
            (q.answers.len(), q.correct.len() > 1, q.correct.clone())
        };
        let mut ch: Vec<usize> = choices.into_iter().filter(|&c| c < n_ans).collect();
        ch.sort_unstable();
        ch.dedup();
        if ch.is_empty() {
            return;
        }
        if !multi {
            ch.truncate(1);
        }
        st.touch();
        let ok = ch == correct;
        let elapsed = now.duration_since(st.q_open).as_millis().min(st.q_time_ms as u128) as u32;
        let streak_after = if ok { st.players[pi].streak + 1 } else { 0 };
        let base = if ok { st.base_points(elapsed) } else { 0 };
        let bonus = if ok && st.settings.streak_bonus { (streak_after - 1).min(5) * 100 } else { 0 };
        let points = base + bonus;
        {
            let p = &mut st.players[pi];
            p.answers[qi] = Some(Ans { choices: ch, ok, points, bonus, ms: elapsed });
            p.score += points;
            p.streak = streak_after;
        }
        let answered = st.answered_count(qi);
        let online = st.online_count();
        let prog = obj! {"t" => "progress", "answered" => answered, "online" => online};
        for (_, tx) in &st.hosts {
            send(tx, &prog);
        }
        let order = st.order();
        if let Some(tx) = &st.players[pi].tx {
            send(tx, &st.player_state(pi, &order));
        }
        if st.all_answered() {
            st.enter_reveal(self);
        }
    }

    pub fn host_cmd(self: &Arc<Self>, c: HostCmd) -> Result<(), String> {
        let mut guard = self.lock();
        let st: &mut Inner = &mut guard;
        st.touch();
        match c {
            HostCmd::Start => {
                if st.phase != Phase::Lobby {
                    return Ok(());
                }
                if st.players.is_empty() {
                    return Err("Belum ada pemain yang bergabung".into());
                }
                st.qi = 0;
                st.enter_read(self);
            }
            HostCmd::Next => match st.phase {
                Phase::Reveal => st.after_reveal(self),
                Phase::Board => st.next_question(self),
                _ => {}
            },
            HostCmd::Skip => match st.phase {
                Phase::Read => st.enter_question(self),
                Phase::Question => st.enter_reveal(self),
                _ => {}
            },
            HostCmd::End => {
                if !matches!(st.phase, Phase::Lobby | Phase::Final) {
                    st.enter_final();
                }
            }
            HostCmd::Reset => {
                if st.phase == Phase::Final {
                    st.questions = prepare(&st.base, &st.settings);
                    let n = st.n();
                    for p in st.players.iter_mut() {
                        p.score = 0;
                        p.streak = 0;
                        p.answers = vec![None; n];
                    }
                    st.players.retain(|p| p.tx.is_some());
                    st.phase = Phase::Lobby;
                    st.qi = 0;
                    st.deadline = None;
                    st.epoch += 1;
                    st.broadcast();
                }
            }
            HostCmd::AddTime(secs) => {
                if st.phase == Phase::Question {
                    let secs = secs.clamp(1, 120);
                    if let Some(d) = st.deadline {
                        let nd = d + Duration::from_secs(secs as u64);
                        st.deadline = Some(nd);
                        st.epoch += 1;
                        self.schedule(st.epoch, nd.saturating_duration_since(Instant::now()));
                        st.broadcast();
                    }
                }
            }
            HostCmd::Kick(id) => {
                if let Some(pos) = st.players.iter().position(|p| p.id == id) {
                    let p = st.players.remove(pos);
                    if let Some(tx) = p.tx {
                        send(&tx, &obj! {"t" => "kicked"});
                        let _ = tx.send(Out::Close);
                    }
                    if st.phase == Phase::Question && st.all_answered() {
                        st.enter_reveal(self);
                    } else {
                        st.broadcast();
                    }
                }
            }
            HostCmd::Lock(l) => {
                st.locked = l;
                st.send_hosts();
            }
        }
        Ok(())
    }
}
