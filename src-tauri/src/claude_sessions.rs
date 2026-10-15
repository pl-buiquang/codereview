//! Discovery of Claude Code sessions on this machine.
//!
//! Live sessions come from `~/.claude/sessions/<pid>.json` (written by Claude Code itself, an
//! undocumented format — parse leniently), which is the only source for the tmux pane. Status,
//! waiting state and job ids are overlaid from `claude agents --json`, the documented interface. Session history comes from the transcripts in
//! `~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TmuxLoc {
    pub session: String,
    pub window: String,
    pub pane: String,
    /// `session:window_index`, as shown by tmux; `None` if the pane is gone.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusState {
    Ready,
    Detached,
    Background,
    NoTerminal,
    NotLive,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeSession {
    pub session_id: String,
    pub pid: Option<i64>,
    pub live: bool,
    pub status: Option<String>,
    pub kind: Option<String>,
    /// A background session currently viewed through a `claude attach` client.
    pub attached: bool,
    /// Background job id taken by `claude attach|stop|logs`.
    pub job_id: Option<String>,
    /// What the session is blocked on (e.g. "input needed", "dialog open").
    pub waiting_for: Option<String>,
    /// For background sessions nobody is attached to: the `claude agents` view best placed to
    /// open it.
    pub agent_view: Option<AgentView>,
    pub name: Option<String>,
    pub cwd: String,
    pub git_branch: Option<String>,
    /// `owner/name` of the cwd's `origin` remote (live sessions only).
    pub repo: Option<String>,
    pub tmux: Option<TmuxLoc>,
    pub tty: Option<String>,
    /// Last activity, epoch milliseconds.
    pub updated_at: i64,
    pub focus: FocusState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    pid: i64,
    session_id: String,
    cwd: String,
    kind: Option<String>,
    status: Option<String>,
    name: Option<String>,
    tmux: Option<String>,
    updated_at: Option<i64>,
    status_updated_at: Option<i64>,
    started_at: Option<i64>,
    /// Pre-started process Claude Code keeps ready for the next background session.
    spare: Option<bool>,
    job_id: Option<String>,
    waiting_for: Option<String>,
}

/// One row of `claude agents --json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntry {
    id: Option<String>,
    session_id: String,
    status: Option<String>,
    waiting_for: Option<String>,
}

/// `claude agents --json`; empty if the CLI is missing or fails.
pub fn agents() -> Vec<AgentEntry> {
    Command::new(crate::tools::claude_bin())
        .args(["agents", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or_default()
}

fn claude_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude"))
}

/// Parses tmux's `"<session>:@<window>.%<pane>"` locator.
pub fn parse_tmux_loc(s: &str) -> Option<TmuxLoc> {
    let (session, rest) = s.rsplit_once(':')?;
    let (window, pane) = rest.split_once('.')?;
    if session.is_empty() || !window.starts_with('@') || !pane.starts_with('%') {
        return None;
    }
    Some(TmuxLoc { session: session.into(), window: window.into(), pane: pane.into(), label: None })
}

fn read_session_files(dir: &Path) -> Vec<SessionFile> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str::<SessionFile>(&s).ok())
        .collect()
}

/// pid → tty (`None` when the process has no controlling terminal) for every live pid.
fn live_pids(pids: &[i64]) -> HashMap<i64, Option<String>> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    let Ok(out) = Command::new("ps").args(["-o", "pid=,tty=", "-p", &list]).output() else {
        return HashMap::new();
    };
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ps(out: &str) -> HashMap<i64, Option<String>> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let tty = it.next().filter(|t| !t.starts_with('?') && *t != "??").map(|t| format!("/dev/{t}"));
            Some((pid, tty))
        })
        .collect()
}

struct Pane {
    session: String,
    window: String,
    /// `session:window_index`
    label: String,
    tty: String,
}

#[derive(Default)]
struct TmuxState {
    panes: HashMap<String, Pane>,
    /// tmux session name → attached client ttys
    clients: HashMap<String, Vec<String>>,
}

fn tmux_state() -> TmuxState {
    let tmux = crate::tools::tmux_bin();
    let run = |args: &[&str]| -> String {
        Command::new(&tmux)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let mut state = TmuxState::default();
    let format = "#{pane_id}\t#{session_name}\t#{window_id}\t#{window_index}\t#{pane_tty}";
    for line in run(&["list-panes", "-a", "-F", format]).lines() {
        if let [pane, session, window, index, tty] = line.split('\t').collect::<Vec<_>>()[..] {
            state.panes.insert(
                pane.into(),
                Pane { session: session.into(), window: window.into(), label: format!("{session}:{index}"), tty: tty.into() },
            );
        }
    }
    for line in run(&["list-clients", "-F", "#{session_name}\t#{client_tty}"]).lines() {
        if let Some((session, tty)) = line.split_once('\t') {
            state.clients.entry(session.into()).or_default().push(tty.into());
        }
    }
    state
}

fn git_info(cwd: &str) -> (Option<String>, Option<String>) {
    let path = Path::new(cwd);
    let branch = crate::git::run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|b| !b.is_empty() && b != "HEAD");
    let remote = crate::git::remote_info(path);
    let repo = remote.owner.zip(remote.name).map(|(o, n)| format!("{o}/{n}"));
    (branch, repo)
}

impl TmuxState {
    fn pane_on_tty(&self, tty: &str) -> Option<TmuxLoc> {
        self.panes.iter().find(|(_, p)| p.tty == tty).map(|(id, p)| TmuxLoc {
            session: p.session.clone(),
            window: p.window.clone(),
            pane: id.clone(),
            label: Some(p.label.clone()),
        })
    }
}

/// A running `claude agents` view (the TUI listing background sessions).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentView {
    pub pid: i64,
    pub cwd: Option<String>,
    /// `--cwd` filter: the view only lists sessions started under this path.
    pub filter_cwd: Option<String>,
    pub tmux: Option<TmuxLoc>,
    pub tty: String,
    pub focus: FocusState,
}

#[derive(Debug, Default, PartialEq)]
struct Clients {
    /// `(session id prefix, tty)` for every `claude attach <id>`.
    attach: Vec<(String, String)>,
    /// `(pid, tty, --cwd filter)` for every interactive `claude agents`.
    views: Vec<(i64, String, Option<String>)>,
}

fn claude_subcommand<'a>(args: &'a [&'a str], sub: &str) -> Option<&'a [&'a str]> {
    let i = args.windows(2).position(|w| (w[0] == "claude" || w[0].ends_with("/claude")) && w[1] == sub)?;
    Some(&args[i + 2..])
}

fn parse_clients(out: &str) -> Clients {
    let mut clients = Clients::default();
    for l in out.lines() {
        let mut it = l.split_whitespace();
        let (Some(pid), Some(tty)) = (it.next().and_then(|p| p.parse::<i64>().ok()), it.next()) else { continue };
        if tty.starts_with('?') {
            continue;
        }
        let tty = format!("/dev/{tty}");
        let args: Vec<&str> = it.collect();
        if let Some(rest) = claude_subcommand(&args, "attach") {
            if let Some(id) = rest.first().filter(|a| !a.starts_with('-') && a.len() >= 4) {
                clients.attach.push((id.to_string(), tty));
            }
        } else if let Some(rest) = claude_subcommand(&args, "agents") {
            if rest.iter().any(|a| *a == "--json" || *a == "--help" || *a == "-h") {
                continue;
            }
            let filter = rest.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--cwd") {
                Some("") => rest.get(i + 1).map(|v| v.to_string()),
                Some(v) => v.strip_prefix('=').map(str::to_string),
                None => None,
            });
            clients.views.push((pid, tty, filter));
        }
    }
    clients
}

fn claude_clients() -> Clients {
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,tty=,command="]).output() else { return Clients::default() };
    parse_clients(&String::from_utf8_lossy(&out.stdout))
}

fn find_transcript(projects: &Path, session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.exists())
}

fn continued_in_from_line(line: &str) -> Option<String> {
    if !line.contains("\"continued-in\"") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    (v.get("type")?.as_str()? == "continued-in")
        .then(|| v.get("continuedInSessionId")?.as_str().map(str::to_string))
        .flatten()
}

/// The session a transcript was handed over to (`continued-in` marker, written when a session
/// is moved to the background). It sits near the end, so only the tail is read.
fn transcript_continued_in(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 64 * 1024;
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    String::from_utf8_lossy(&buf).lines().rev().find_map(continued_in_from_line)
}

pub(crate) fn process_cwd(pid: i64) -> Option<String> {
    if let Ok(p) = fs::read_link(format!("/proc/{pid}/cwd")) {
        return Some(p.to_string_lossy().into_owned());
    }
    let out = Command::new("lsof").args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.strip_prefix('n').map(str::to_string))
}

/// All running agent views.
pub fn agent_views() -> Vec<AgentView> {
    let views = claude_clients().views;
    if views.is_empty() {
        return Vec::new();
    }
    let tmux = tmux_state();
    views
        .into_iter()
        .map(|(pid, tty, filter_cwd)| {
            let loc = tmux.pane_on_tty(&tty);
            let focus = focus_state(None, loc.as_ref(), Some(&tty), &tmux);
            AgentView { pid, cwd: process_cwd(pid), filter_cwd, tmux: loc, tty, focus }
        })
        .collect()
}

/// The agent view to open a background session from: it must list the session (its `--cwd`
/// filter, if any, contains the session's cwd); focusable views first, then the one whose own cwd
/// is the deepest ancestor of the session's cwd.
pub fn pick_agent_view(views: &[AgentView], session_cwd: &str) -> Option<AgentView> {
    let session = Path::new(session_cwd);
    views
        .iter()
        .rev() // max_by_key keeps the last maximum; reversed, the first listed view wins ties
        .filter(|v| v.filter_cwd.as_deref().is_none_or(|f| session.starts_with(f)))
        .max_by_key(|v| {
            let depth = v.cwd.as_deref().filter(|c| session.starts_with(c)).map_or(0, |c| Path::new(c).components().count());
            (v.focus == FocusState::Ready, depth)
        })
        .cloned()
}

/// All live sessions, most recently active first.
pub fn live_sessions() -> Vec<ClaudeSession> {
    live_sessions_from(&agents())
}

/// Live sessions, plus the ids of stale fork parents that were left out of them.
pub fn live_sessions_and_hidden(agents: &[AgentEntry]) -> (Vec<ClaudeSession>, HashSet<String>) {
    let Some(dir) = claude_dir() else { return (Vec::new(), HashSet::new()) };
    let by_session: HashMap<&str, &AgentEntry> = agents.iter().map(|a| (a.session_id.as_str(), a)).collect();
    let files = read_session_files(&dir.join("sessions"));
    let alive = live_pids(&files.iter().map(|f| f.pid).collect::<Vec<_>>());
    let tmux = tmux_state();
    let has_bg = files.iter().any(|f| f.kind.as_deref() == Some("bg"));
    let attach = if has_bg { claude_clients().attach } else { Vec::new() };
    let mut views = if has_bg { agent_views() } else { Vec::new() };

    let mut seen = HashSet::new();
    let mut files: Vec<_> =
        files.into_iter().filter(|f| alive.contains_key(&f.pid) && f.spare != Some(true)).collect();
    // A process whose session was handed off (moved to the background) has switched to the agent
    // view, showing the continuation; its session file still describes the old session.
    let projects = dir.join("projects");
    let mut stale = HashSet::new();
    let mut handoff_views: HashMap<String, AgentView> = HashMap::new();
    for f in &files {
        let Some(next) = find_transcript(&projects, &f.session_id).and_then(|p| transcript_continued_in(&p)) else {
            continue;
        };
        if next == f.session_id {
            continue;
        }
        stale.insert(f.session_id.clone());
        let tty = alive.get(&f.pid).cloned().flatten();
        let loc = f.tmux.as_deref().and_then(parse_tmux_loc).map(|mut loc| {
            loc.label = tmux.panes.get(&loc.pane).map(|p| p.label.clone());
            loc
        });
        let focus = focus_state(None, loc.as_ref(), tty.as_deref(), &tmux);
        let view = AgentView { pid: f.pid, cwd: Some(f.cwd.clone()), filter_cwd: None, tmux: loc, tty: tty.unwrap_or_default(), focus };
        views.push(view.clone());
        handoff_views.insert(next, view);
    }
    files.retain(|f| !stale.contains(&f.session_id));
    files.sort_by_key(|f| std::cmp::Reverse(updated_ms(f)));

    let sessions = files
        .into_iter()
        .filter(|f| seen.insert(f.session_id.clone()))
        .map(|f| {
            let mut tty = alive.get(&f.pid).cloned().flatten();
            let mut tmux_loc = f.tmux.as_deref().and_then(parse_tmux_loc).map(|mut loc| {
                loc.label = tmux.panes.get(&loc.pane).map(|p| p.label.clone());
                loc
            });
            let client_tty = (f.kind.as_deref() == Some("bg"))
                .then(|| attach.iter().find(|(prefix, _)| f.session_id.starts_with(prefix.as_str())))
                .flatten()
                .map(|(_, t)| t.clone());
            let attached = client_tty.is_some();
            if let Some(t) = client_tty {
                tmux_loc = tmux.pane_on_tty(&t);
                tty = Some(t);
            }
            let kind = if attached { None } else { f.kind.as_deref() };
            let agent_view = (kind == Some("bg"))
                .then(|| handoff_views.get(&f.session_id).cloned().or_else(|| pick_agent_view(&views, &f.cwd)))
                .flatten();
            let focus = focus_state(kind, tmux_loc.as_ref(), tty.as_deref(), &tmux);
            let (git_branch, repo) = git_info(&f.cwd);
            let agent = by_session.get(f.session_id.as_str());
            ClaudeSession {
                updated_at: updated_ms(&f),
                pid: Some(f.pid),
                live: true,
                status: agent.and_then(|a| a.status.clone()).or(f.status),
                kind: f.kind,
                attached,
                job_id: agent.and_then(|a| a.id.clone()).or(f.job_id),
                waiting_for: agent.and_then(|a| a.waiting_for.clone()).or(f.waiting_for).filter(|w| !w.is_empty()),
                agent_view,
                session_id: f.session_id,
                name: f.name,
                cwd: f.cwd,
                git_branch,
                repo,
                tmux: tmux_loc,
                tty,
                focus,
            }
        })
        .collect();
    (sessions, stale)
}

pub fn live_sessions_from(agents: &[AgentEntry]) -> Vec<ClaudeSession> {
    live_sessions_and_hidden(agents).0
}

fn updated_ms(f: &SessionFile) -> i64 {
    f.status_updated_at.or(f.updated_at).or(f.started_at).unwrap_or(0)
}

fn focus_state(kind: Option<&str>, tmux_loc: Option<&TmuxLoc>, tty: Option<&str>, tmux: &TmuxState) -> FocusState {
    if kind == Some("bg") {
        return FocusState::Background;
    }
    match tmux_loc {
        Some(loc) if tmux.clients.get(&loc.session).is_some_and(|c| !c.is_empty()) => FocusState::Ready,
        Some(_) => FocusState::Detached,
        None if tty.is_some() => FocusState::Ready,
        None => FocusState::NoTerminal,
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct TranscriptInfo {
    custom_title: Option<String>,
    agent_name: Option<String>,
    ai_title: Option<String>,
    last_prompt: Option<String>,
    cwd: Option<String>,
    git_branch: Option<String>,
    continued_in: Option<String>,
}

impl TranscriptInfo {
    fn name(&self) -> Option<String> {
        self.custom_title
            .clone()
            .or_else(|| self.agent_name.clone())
            .or_else(|| self.ai_title.clone())
            .or_else(|| self.last_prompt.as_ref().map(|p| truncate(p, 80)))
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// Transcripts are append-only and can be several MB, so only lines that can carry a field we
/// need are JSON-parsed. Later title entries win.
fn parse_transcript(reader: impl BufRead) -> TranscriptInfo {
    let mut info = TranscriptInfo::default();
    for line in reader.lines().map_while(Result::ok) {
        if let Some(next) = continued_in_from_line(&line) {
            info.continued_in = Some(next);
            continue;
        }
        let wanted = ["\"custom-title\"", "\"agent-name\"", "\"ai-title\"", "\"last-prompt\""]
            .iter()
            .any(|t| line.contains(t))
            || (info.cwd.is_none() && line.contains("\"cwd\""));
        if !wanted {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        match v.get("type").and_then(|t| t.as_str()) {
            Some("custom-title") => info.custom_title = s("customTitle").or(info.custom_title),
            Some("agent-name") => info.agent_name = s("agentName").or(info.agent_name),
            Some("ai-title") => info.ai_title = s("aiTitle").or(info.ai_title),
            Some("last-prompt") => info.last_prompt = s("lastPrompt").or(info.last_prompt),
            _ => {}
        }
        if info.cwd.is_none() {
            if let Some(cwd) = s("cwd") {
                info.cwd = Some(cwd);
                info.git_branch = s("gitBranch").filter(|b| !b.is_empty() && b != "HEAD");
            }
        }
    }
    info
}

static TRANSCRIPT_CACHE: Mutex<Option<HashMap<PathBuf, (SystemTime, TranscriptInfo)>>> = Mutex::new(None);

fn transcript_info(path: &Path, mtime: SystemTime) -> Option<TranscriptInfo> {
    let mut guard = TRANSCRIPT_CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((m, info)) = cache.get(path) {
        if *m == mtime {
            return Some(info.clone());
        }
    }
    let file = fs::File::open(path).ok()?;
    let info = parse_transcript(BufReader::new(file));
    cache.insert(path.to_path_buf(), (mtime, info.clone()));
    Some(info)
}

/// Non-live sessions whose transcript was written within the last `days` days.
pub fn history_sessions(days: u32, exclude: &HashSet<String>) -> Vec<ClaudeSession> {
    let Some(dir) = claude_dir() else { return Vec::new() };
    let cutoff = SystemTime::now() - Duration::from_secs(u64::from(days) * 86_400);
    let Ok(projects) = fs::read_dir(dir.join("projects")) else { return Vec::new() };

    let mut out = Vec::new();
    for project in projects.flatten() {
        let Ok(files) = fs::read_dir(project.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            if exclude.contains(&session_id) {
                continue;
            }
            let Some(mtime) = f.metadata().ok().and_then(|m| m.modified().ok()) else { continue };
            if mtime < cutoff {
                continue;
            }
            let Some(info) = transcript_info(&path, mtime) else { continue };
            if info.continued_in.is_some() {
                continue;
            }
            let Some(cwd) = info.cwd.clone() else { continue };
            out.push(ClaudeSession {
                session_id,
                pid: None,
                live: false,
                status: None,
                kind: None,
                attached: false,
                job_id: None,
                waiting_for: None,
                agent_view: None,
                name: info.name(),
                cwd,
                git_branch: info.git_branch,
                repo: None,
                tmux: None,
                tty: None,
                updated_at: mtime.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
                focus: FocusState::NotLive,
            });
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tmux_locator() {
        let loc = parse_tmux_loc("0:@7.%7").unwrap();
        assert_eq!((loc.session.as_str(), loc.window.as_str(), loc.pane.as_str()), ("0", "@7", "%7"));
        assert_eq!(parse_tmux_loc("my:sess:@12.%30").unwrap().session, "my:sess");
        assert!(parse_tmux_loc("garbage").is_none());
        assert!(parse_tmux_loc("0:7.7").is_none());
    }

    #[test]
    fn parses_ps_output() {
        let m = parse_ps("  123 ttys000\n  456 ??\n 789 pts/3\n");
        assert_eq!(m[&123].as_deref(), Some("/dev/ttys000"));
        assert_eq!(m[&456], None);
        assert_eq!(m[&789].as_deref(), Some("/dev/pts/3"));
    }

    #[test]
    fn parses_agents() {
        let agents: Vec<AgentEntry> = serde_json::from_str(
            r#"[{"id":"84c6c6c4","cwd":"/f","kind":"background","startedAt":5,"sessionId":"s-parked","state":"blocked"},
                {"pid":1,"id":"a3f9e7bf","cwd":"/a","kind":"background","sessionId":"s-live","status":"waiting","waitingFor":"permission prompt"}]"#,
        )
        .unwrap();
        assert_eq!(agents[0].id.as_deref(), Some("84c6c6c4"));
        assert_eq!(agents[1].status.as_deref(), Some("waiting"));
        assert_eq!(agents[1].waiting_for.as_deref(), Some("permission prompt"));
    }

    #[test]
    fn parses_attach_clients_and_agent_views() {
        let ps = "1 ttys003  /Users/me/.local/bin/claude attach fa14b19c\n\
                  2 ttys004  -zsh\n\
                  3 ??       claude attach abcd1234\n\
                  4 ttys005  claude attach --help\n\
                  5 ttys006  vim claude attach.md\n\
                  6 ttys007  /Users/me/.local/bin/claude agents\n\
                  7 ttys008  claude agents --cwd /a/b\n\
                  8 ttys009  claude agents --cwd=/c\n\
                  9 ttys010  claude agents --json\n\
                  10 ??      claude agents\n";
        let c = parse_clients(ps);
        assert_eq!(c.attach, vec![("fa14b19c".to_string(), "/dev/ttys003".to_string())]);
        assert_eq!(
            c.views,
            vec![
                (6, "/dev/ttys007".to_string(), None),
                (7, "/dev/ttys008".to_string(), Some("/a/b".to_string())),
                (8, "/dev/ttys009".to_string(), Some("/c".to_string())),
            ]
        );
    }

    #[test]
    fn reads_continued_in_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.jsonl");
        let filler = format!("{{\"type\":\"user\",\"cwd\":\"/x\",\"pad\":\"{}\"}}\n", "a".repeat(100_000));
        let tail = concat!(
            r#"{"type":"system","content":"Backgrounding after the current tool finishes…"}"#, "\n",
            r#"{"type":"continued-in","sessionId":"old","continuedInSessionId":"new"}"#, "\n",
            r#"{"type":"cost-state"}"#, "\n",
        );
        fs::write(&path, format!("{filler}{tail}")).unwrap();
        assert_eq!(transcript_continued_in(&path).as_deref(), Some("new"));

        let plain = dir.path().join("plain.jsonl");
        fs::write(&plain, r#"{"type":"user","cwd":"/x"}"#).unwrap();
        assert_eq!(transcript_continued_in(&plain), None);

        let info = parse_transcript(format!("{filler}{tail}").as_bytes());
        assert_eq!(info.continued_in.as_deref(), Some("new"));
        assert_eq!(info.cwd.as_deref(), Some("/x"));
    }

    fn view(pid: i64, cwd: &str, filter: Option<&str>, focus: FocusState) -> AgentView {
        AgentView { pid, cwd: Some(cwd.into()), filter_cwd: filter.map(Into::into), tmux: None, tty: "/dev/x".into(), focus }
    }

    #[test]
    fn picks_agent_view() {
        let views = vec![
            view(1, "/home/me", None, FocusState::Ready),
            view(2, "/home/me/projects", None, FocusState::Ready),
            view(3, "/home/me/projects/app", None, FocusState::Detached),
            view(4, "/elsewhere", Some("/other"), FocusState::Ready),
        ];
        assert_eq!(pick_agent_view(&views, "/home/me/projects/app/sub").unwrap().pid, 2);
        assert_eq!(pick_agent_view(&views, "/tmp").unwrap().pid, 1);
        assert_eq!(pick_agent_view(&views[3..], "/tmp"), None);
        assert_eq!(pick_agent_view(&views[3..], "/other/x").unwrap().pid, 4);
        assert!(pick_agent_view(&[], "/x").is_none());
    }

    #[test]
    fn parses_session_file() {
        let f: SessionFile = serde_json::from_str(
            r#"{"pid":27907,"sessionId":"abc","cwd":"/x","kind":"interactive","tmux":"0:@7.%7",
                "name":"n","status":"busy","updatedAt":2,"statusUpdatedAt":3,"extra":true,
                "spare":true,"jobId":"abc12345","waitingFor":"input needed"}"#,
        )
        .unwrap();
        assert_eq!(f.spare, Some(true));
        assert_eq!(f.job_id.as_deref(), Some("abc12345"));
        assert_eq!(f.waiting_for.as_deref(), Some("input needed"));
        assert_eq!(updated_ms(&f), 3);
        assert_eq!(f.tmux.as_deref(), Some("0:@7.%7"));
    }

    #[test]
    fn focus_states() {
        let mut tmux = TmuxState::default();
        let loc = parse_tmux_loc("0:@1.%1").unwrap();
        assert_eq!(focus_state(Some("bg"), None, None, &tmux), FocusState::Background);
        assert_eq!(focus_state(Some("interactive"), Some(&loc), None, &tmux), FocusState::Detached);
        tmux.clients.insert("0".into(), vec!["/dev/ttys000".into()]);
        assert_eq!(focus_state(Some("interactive"), Some(&loc), None, &tmux), FocusState::Ready);
        assert_eq!(focus_state(None, None, Some("/dev/ttys001"), &tmux), FocusState::Ready);
        assert_eq!(focus_state(None, None, None, &tmux), FocusState::NoTerminal);
    }

    #[test]
    fn transcript_title_priority_and_cwd() {
        let jsonl = [
            r#"{"type":"user","cwd":"/repo","gitBranch":"feat/x","message":{}}"#,
            r#"{"type":"user","cwd":"/other","gitBranch":"main"}"#,
            r#"{"type":"ai-title","aiTitle":"AI"}"#,
            r#"{"type":"agent-name","agentName":"old"}"#,
            r#"{"type":"agent-name","agentName":"new"}"#,
            r#"{"type":"last-prompt","lastPrompt":"do it"}"#,
        ]
        .join("\n");
        let info = parse_transcript(jsonl.as_bytes());
        assert_eq!(info.cwd.as_deref(), Some("/repo"));
        assert_eq!(info.git_branch.as_deref(), Some("feat/x"));
        assert_eq!(info.name().as_deref(), Some("new"));

        let only_prompt = parse_transcript(r#"{"type":"last-prompt","lastPrompt":"hello\nworld"}"#.as_bytes());
        assert_eq!(only_prompt.name().as_deref(), Some("hello world"));
    }
}
