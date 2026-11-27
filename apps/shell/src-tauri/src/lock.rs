//! The origin lock's state: which servers are open to the window, which page it shows, the
//! app's own way back to the picker, and how often something opens outside the app.
//!
//! The rule of where a URL belongs is [`navigation::judge`]; this decides what the window does
//! with it, given where the window is.
//!
//! **The app's own pages.** Only the picker reaches the app's commands, and on Linux the
//! webview names the page a message comes from by the URL the window is loading, not by the
//! page that sent it. So a server's page must never send the window to the app's origin: while
//! that load ran, the server's page would still be there, and its messages would pass as the
//! picker's. A navigation to the app's origin goes ahead only from the app's own pages, or as
//! the first page the window loads, or when the app itself shows the picker. That it does in
//! two steps: it loads `about:blank`, which replaces the server's page, and only once that
//! blank page has committed, the picker. When a server's page asks for the app's origin, by a
//! link, a redirect or the history, as iOS's back gesture from the server's first page does,
//! the navigation is cancelled and the app shows the picker in those two steps instead.
//! WebView2, on Windows, names each message's own page, and the picker loads at once there.
//!
//! The commands check, besides, that the page last committed in the window is the app's own
//! ([`Lock::on_app_page`]): a server's page that is still there while the picker loads gets
//! nothing, whatever URL its messages carry.
//!
//! **Opening outside.** No webview tells the app whether a click started a navigation, nor
//! whether it is the main frame's. So a navigation that would open outside the app does so at
//! most once a second, not again for the same URL within ten seconds, and never in the first
//! second of a page's load, when only a script or a frame could have asked for it.
//!
//! **Opening a server.** Each attempt to open one, from the picker's command to the server's
//! first page, has a number. The window asks for the server's page only if the attempt is
//! still the latest, and once it has, the attempt is pending until a page starts loading. The
//! picker can cancel it, and a page that never starts gives up after a time limit: either way,
//! a pending page load is stopped by showing the picker again ([`Lock::cancel_opening`],
//! [`Lock::stalled`]). A newer attempt, and any way back to the picker, ends the older one.
//!
//! **A page no hook saw.** Android asks the app about no form posted with POST, so a form on
//! a server's page that posts to another origin would load it in the window. Each page that
//! starts loading in the window is judged again: one that is not the app's or a server's is
//! left, opening outside as a navigation to it would, and the window goes back to the server's
//! last page.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use url::Url;

use crate::navigation::{self, Origin, Place, Verdict};

/// How long after one URL opens outside the app the next one may.
pub const OUTSIDE_INTERVAL: Duration = Duration::from_secs(1);
/// How long the same URL does not open outside again.
pub const OUTSIDE_REPEAT: Duration = Duration::from_secs(10);
/// How long after a page starts loading nothing opens outside the app.
pub const PAGE_SETTLE: Duration = Duration::from_secs(1);

/// The page every way back to the picker loads first.
pub const BLANK: &str = "about:blank";

/// What the window does with a navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    /// It goes ahead.
    Load,
    /// It is cancelled.
    Refuse,
    /// It is cancelled, and the URL opens in the system's browser, or the app for its scheme.
    OpenOutside(Url),
    /// It is cancelled, as it would have opened outside the app too soon after another one, or
    /// after the page started loading.
    TooSoon,
    /// It is cancelled, and the app shows the picker itself: a server's page asked for the
    /// app's origin.
    ShowPicker,
}

impl Navigation {
    /// Whether the webview goes ahead with it.
    pub fn goes_ahead(&self) -> bool {
        *self == Self::Load
    }
}

impl fmt::Display for Navigation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Load => "loaded",
            Self::Refuse => "refused",
            Self::OpenOutside(_) => "opened outside the app",
            Self::TooSoon => "refused, as it would open outside the app too soon",
            Self::ShowPicker => "refused, and the picker shown in its place",
        })
    }
}

/// What the window does once a page has started loading in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Started {
    /// Nothing more.
    Stay,
    /// It loads the picker: the blank page that replaced the server's has committed.
    LoadPicker(Url),
    /// It leaves a page the lock does not allow, which no navigation hook saw: it opens
    /// `outside` outside the app, if anything, and goes `back` to the server's last page, or
    /// with none to the picker.
    Leave {
        outside: Option<Url>,
        back: Option<Url>,
    },
}

/// The kind of page the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// None yet: the window is loading its first.
    Start,
    /// The app's own, such as the picker.
    App,
    /// `about:blank`.
    Blank,
    /// A server's.
    Server,
    /// One the lock does not allow, which the window is leaving.
    Elsewhere,
}

/// Where the app's own way back to the picker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Showing {
    No,
    /// The blank page is loading.
    Blank,
    /// The blank page has committed, and the picker is loading.
    Picker,
}

#[derive(Debug)]
struct State {
    /// The origins of the servers opened since the app started, the one in use last.
    servers: Vec<Origin>,
    page: Page,
    showing: Showing,
    /// The server's page the window last showed, to go back to.
    last_server_page: Option<Url>,
    /// When the page last started loading.
    page_started: Option<Instant>,
    /// What opened outside the app last, and when.
    last_outside: Option<(Instant, Url)>,
    /// The number of the latest attempt to open a server.
    attempt: Attempt,
    /// The attempt whose server's page the window has asked for and which has not started
    /// loading yet, with that server.
    pending: Option<(Attempt, Origin)>,
}

/// The number of an attempt to open a server.
pub type Attempt = u64;

/// The origin lock of the app's window.
#[derive(Debug)]
pub struct Lock {
    app: Origin,
    picker: Url,
    /// Whether the way back to the picker loads a blank page first.
    through_blank: bool,
    state: Mutex<State>,
}

impl Lock {
    /// A lock with no server open yet, for a window whose first page is `picker`, on the app's
    /// own origin `app`; `through_blank` where the webview names a message's page by the URL
    /// the window is loading, or may.
    pub fn new(app: Origin, picker: Url, through_blank: bool) -> Self {
        Self {
            app,
            picker,
            through_blank,
            state: Mutex::new(State {
                servers: Vec::new(),
                page: Page::Start,
                showing: Showing::No,
                last_server_page: None,
                page_started: None,
                last_outside: None,
                attempt: 0,
                pending: None,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The picker's URL.
    pub fn picker(&self) -> &Url {
        &self.picker
    }

    /// Where `url` belongs: the rule alone, with the servers open now.
    pub fn judge(&self, url: &Url) -> Verdict {
        navigation::judge(url, &self.app, &self.state().servers)
    }

    /// Starts an attempt to open a server, which ends any earlier one.
    pub fn begin_opening(&self) -> Attempt {
        let mut state = self.state();
        state.attempt += 1;
        state.pending = None;
        state.attempt
    }

    /// Opens the lock to `origin`, the server the window is about to load for `attempt`, and
    /// notes the page as pending; or, when the attempt was cancelled or another began since,
    /// does nothing and says so.
    pub fn open_server(&self, origin: Origin, attempt: Attempt) -> bool {
        let mut state = self.state();
        if state.attempt != attempt {
            return false;
        }
        state.servers.retain(|opened| *opened != origin);
        state.servers.push(origin.clone());
        state.pending = Some((attempt, origin));
        true
    }

    /// Whether `attempt` is still the latest attempt to open a server.
    pub fn still_opening(&self, attempt: Attempt) -> bool {
        self.state().attempt == attempt
    }

    /// Cancels the attempt to open a server under way: the server whose page the window is
    /// loading, which showing the picker stops, or none, when the attempt had not got that far.
    pub fn cancel_opening(&self) -> Option<Origin> {
        let mut state = self.state();
        state.attempt += 1;
        state.pending.take().map(|(_, origin)| origin)
    }

    /// Gives up on `attempt` if its server's page has not started loading yet: the server,
    /// whose page load showing the picker stops.
    pub fn stalled(&self, attempt: Attempt) -> Option<Origin> {
        let mut state = self.state();
        if !state
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == attempt)
        {
            return None;
        }
        state.attempt += 1;
        state.pending.take().map(|(_, origin)| origin)
    }

    /// Closes the lock to `origin`, a server removed from the list.
    pub fn forget(&self, origin: &Origin) {
        let mut state = self.state();
        state.servers.retain(|opened| opened != origin);
        if state
            .last_server_page
            .as_ref()
            .and_then(Origin::of)
            .is_some_and(|last| last == *origin)
        {
            state.last_server_page = None;
        }
    }

    /// What the window does with a navigation to `url` at `now`.
    pub fn navigation(&self, url: &Url, now: Instant) -> Navigation {
        let mut state = self.state();
        match navigation::judge(url, &self.app, &state.servers) {
            Verdict::Load(Place::Server) => Navigation::Load,
            Verdict::Load(Place::App) if url.scheme() == "about" => Navigation::Load,
            Verdict::Load(Place::App) => {
                if state.showing == Showing::Picker || matches!(state.page, Page::Start | Page::App)
                {
                    Navigation::Load
                } else if state.showing == Showing::Blank {
                    // The app is already on its way to the picker.
                    Navigation::Refuse
                } else {
                    Navigation::ShowPicker
                }
            }
            Verdict::OpenOutside => {
                if may_open_outside(&mut state, url, now) {
                    Navigation::OpenOutside(url.clone())
                } else {
                    Navigation::TooSoon
                }
            }
            Verdict::Refuse => Navigation::Refuse,
        }
    }

    /// Starts the app's own way back to the picker: the URL the window loads now, the blank
    /// page. The picker follows once that has committed, from [`Lock::started`]. Without the
    /// blank page, the picker at once.
    pub fn show_picker(&self) -> Url {
        let mut state = self.state();
        state.attempt += 1;
        state.pending = None;
        match Url::parse(BLANK) {
            Ok(blank) if self.through_blank => {
                state.showing = Showing::Blank;
                blank
            }
            _ => {
                state.showing = Showing::Picker;
                self.picker.clone()
            }
        }
    }

    /// Whether the page last committed in the window is the app's own, or none has yet: the
    /// only time the picker's commands may run.
    pub fn on_app_page(&self) -> bool {
        matches!(self.state().page, Page::Start | Page::App)
    }

    /// What the window does once a page at `url` has started loading in it, at `now`.
    pub fn started(&self, url: &Url, now: Instant) -> Started {
        let mut state = self.state();
        // Whatever it is, the page asked for to open a server is no longer pending.
        state.pending = None;
        let verdict = navigation::judge(url, &self.app, &state.servers);
        let started = match verdict {
            Verdict::Load(Place::App) if url.scheme() == "about" => {
                state.page = Page::Blank;
                if state.showing == Showing::Blank {
                    state.showing = Showing::Picker;
                    Started::LoadPicker(self.picker.clone())
                } else {
                    Started::Stay
                }
            }
            Verdict::Load(Place::App) => {
                state.page = Page::App;
                state.showing = Showing::No;
                Started::Stay
            }
            Verdict::Load(Place::Server) => {
                state.page = Page::Server;
                state.showing = Showing::No;
                state.last_server_page = Some(url.clone());
                Started::Stay
            }
            Verdict::OpenOutside | Verdict::Refuse => {
                state.page = Page::Elsewhere;
                let outside = (verdict == Verdict::OpenOutside
                    && may_open_outside(&mut state, url, now))
                .then(|| url.clone());
                Started::Leave {
                    outside,
                    back: state.last_server_page.clone(),
                }
            }
        };
        state.page_started = Some(now);
        started
    }
}

/// Whether `url` may open outside the app at `now`, noting it when it may.
fn may_open_outside(state: &mut State, url: &Url, now: Instant) -> bool {
    let since = |then: Instant| now.saturating_duration_since(then);
    if state
        .page_started
        .is_some_and(|started| since(started) < PAGE_SETTLE)
    {
        return false;
    }
    if let Some((then, last)) = &state.last_outside
        && (since(*then) < OUTSIDE_INTERVAL || (last == url && since(*then) < OUTSIDE_REPEAT))
    {
        return false;
    }
    state.last_outside = Some((now, url.clone()));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const PICKER: &str = "tauri://localhost/index.html";
    const SERVER: &str = "https://neon.example.com";

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn origin(text: &str) -> Origin {
        Origin::of(&url(text)).unwrap()
    }

    /// Opens the lock to `server`, as opening it from the picker does.
    fn open(lock: &Lock, server: &str) {
        assert!(lock.open_server(origin(server), lock.begin_opening()));
    }

    /// A lock whose window shows the picker, as the app starts.
    fn lock() -> (Lock, Instant) {
        let lock = Lock::new(origin(PICKER), url(PICKER), true);
        let start = Instant::now();
        assert_eq!(lock.navigation(&url(PICKER), start), Navigation::Load);
        assert_eq!(lock.started(&url(PICKER), start), Started::Stay);
        (lock, start)
    }

    /// The lock, with the window on `page` of the server, a while after `start`.
    fn on_server(page: &str) -> (Lock, Instant) {
        let (lock, start) = lock();
        open(&lock, SERVER);
        let now = start + Duration::from_secs(5);
        assert_eq!(lock.navigation(&url(page), now), Navigation::Load);
        assert_eq!(lock.started(&url(page), now), Started::Stay);
        (lock, now + Duration::from_secs(60))
    }

    #[test]
    fn the_first_page_and_the_pickers_own_navigations_load() {
        let lock = Lock::new(origin(PICKER), url(PICKER), true);
        let now = Instant::now();
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Load);
        lock.started(&url(PICKER), now);
        // The picker reloading itself, and a page of the app's beside it.
        for page in [PICKER, "tauri://localhost/THIRD_PARTY_NOTICES.txt"] {
            assert_eq!(lock.navigation(&url(page), now), Navigation::Load, "{page}");
        }
    }

    #[test]
    fn a_servers_page_cannot_send_the_window_to_the_app() {
        let (lock, now) = on_server("https://neon.example.com/w/acme");
        // A link or a script, a redirect such as /slow answering 302 to the picker, or the
        // history: each is cancelled, and the app shows the picker in its own way.
        for page in [
            PICKER,
            "tauri://localhost/",
            "tauri://localhost/index.html?x=1#y",
        ] {
            assert_eq!(
                lock.navigation(&url(page), now),
                Navigation::ShowPicker,
                "{page}"
            );
        }
    }

    #[test]
    fn the_picker_loads_only_once_a_blank_page_has_replaced_the_servers() {
        let (lock, now) = on_server("https://neon.example.com/w/acme");
        let blank = lock.show_picker();
        assert_eq!(blank.as_str(), BLANK);
        assert_eq!(lock.navigation(&blank, now), Navigation::Load);
        // The server's page is still there until the blank page commits: it may not go to the
        // app's origin, and asks for nothing more.
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Refuse);
        assert_eq!(
            lock.started(&blank, now),
            Started::LoadPicker(url(PICKER)),
            "the blank page is in"
        );
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Load);
        assert_eq!(lock.started(&url(PICKER), now), Started::Stay);
        // On the picker again, its own navigations go ahead.
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Load);
    }

    #[test]
    fn without_the_blank_page_the_picker_loads_at_once() {
        let lock = Lock::new(origin(PICKER), url(PICKER), false);
        let now = Instant::now();
        lock.started(&url(PICKER), now);
        open(&lock, SERVER);
        lock.started(&url(SERVER), now);
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::ShowPicker);
        assert_eq!(lock.show_picker(), url(PICKER));
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Load);
        assert!(!lock.on_app_page(), "the server's page is still there");
        assert_eq!(lock.started(&url(PICKER), now), Started::Stay);
        assert!(lock.on_app_page());
    }

    #[test]
    fn the_commands_run_only_while_the_app_shows_its_own_page() {
        let lock = Lock::new(origin(PICKER), url(PICKER), true);
        let now = Instant::now();
        assert!(lock.on_app_page(), "the first page, as the window opens");
        lock.started(&url(PICKER), now);
        assert!(lock.on_app_page());
        open(&lock, SERVER);
        lock.started(&url(SERVER), now);
        assert!(!lock.on_app_page());
        // On the way back, until the picker itself has committed.
        let blank = lock.show_picker();
        assert!(!lock.on_app_page());
        lock.started(&blank, now);
        assert!(!lock.on_app_page());
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::Load);
        assert!(!lock.on_app_page(), "the picker is only loading");
        lock.started(&url(PICKER), now);
        assert!(lock.on_app_page());
        // A page of another origin that slipped in is not the app's either.
        lock.started(&url("https://evil.example/"), now);
        assert!(!lock.on_app_page());
    }

    #[test]
    fn back_walks_the_servers_history_before_it_reaches_the_picker() {
        // iOS's edge swipe asks the lock about each entry it goes back to, as a navigation.
        let (lock, now) = on_server("https://neon.example.com/w/acme/p/1");
        // The entries the server's pages added: an open panel's, with a query, a fragment or
        // none, and the pages before.
        for entry in [
            "https://neon.example.com/w/acme/p/1?panel=comments",
            "https://neon.example.com/w/acme/p/1#drawer",
            "https://neon.example.com/w/acme/p/1",
            "https://neon.example.com/w/acme",
            "https://neon.example.com/",
        ] {
            assert_eq!(
                lock.navigation(&url(entry), now),
                Navigation::Load,
                "{entry}"
            );
            assert_eq!(lock.started(&url(entry), now), Started::Stay, "{entry}");
        }
        // Past the server's first page, the picker, which the app shows its own way.
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::ShowPicker);
    }

    #[test]
    fn a_blank_page_the_server_loaded_leads_nowhere() {
        let (lock, now) = on_server("https://neon.example.com/");
        assert_eq!(lock.navigation(&url(BLANK), now), Navigation::Load);
        assert_eq!(lock.started(&url(BLANK), now), Started::Stay);
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::ShowPicker);
    }

    #[test]
    fn a_servers_page_that_commits_ends_the_way_back() {
        let (lock, now) = on_server("https://neon.example.com/");
        let blank = lock.show_picker();
        // The server's page got there first: the way back is given up, not left half open.
        assert_eq!(
            lock.started(&url("https://neon.example.com/x"), now),
            Started::Stay
        );
        assert_eq!(lock.started(&blank, now), Started::Stay);
        assert_eq!(lock.navigation(&url(PICKER), now), Navigation::ShowPicker);
    }

    #[test]
    fn the_servers_opened_load_until_removed() {
        let (lock, now) = lock();
        let later = now + Duration::from_secs(60);
        let other = "https://other.example/";
        assert!(matches!(
            lock.navigation(&url(SERVER), later),
            Navigation::OpenOutside(_)
        ));
        open(&lock, SERVER);
        open(&lock, other);
        for page in [SERVER, other] {
            assert_eq!(
                lock.navigation(&url(page), later),
                Navigation::Load,
                "{page}"
            );
        }
        lock.forget(&origin(SERVER));
        assert_eq!(lock.navigation(&url(other), later), Navigation::Load);
        assert!(matches!(
            lock.navigation(&url("https://neon.example.com/w"), later + OUTSIDE_REPEAT),
            Navigation::OpenOutside(_)
        ));
    }

    #[test]
    fn other_origins_open_outside_at_most_once_a_second() {
        let (lock, now) = on_server("https://neon.example.com/");
        let first = url("https://example.com/a");
        assert_eq!(
            lock.navigation(&first, now),
            Navigation::OpenOutside(first.clone())
        );
        // A burst, as a script or a page full of frames would make: nothing more opens.
        for at in [0, 10, 500, 999] {
            let soon = now + Duration::from_millis(at);
            assert_eq!(
                lock.navigation(&url("https://example.org/b"), soon),
                Navigation::TooSoon,
                "{at} ms"
            );
        }
        let second = url("https://example.org/b");
        assert_eq!(
            lock.navigation(&second, now + OUTSIDE_INTERVAL),
            Navigation::OpenOutside(second)
        );
    }

    #[test]
    fn the_same_url_does_not_open_outside_twice_in_ten_seconds() {
        let (lock, now) = on_server("https://neon.example.com/");
        let link = url("mailto:ada@example.com");
        assert_eq!(
            lock.navigation(&link, now),
            Navigation::OpenOutside(link.clone())
        );
        for at in [OUTSIDE_INTERVAL, OUTSIDE_REPEAT - Duration::from_millis(1)] {
            assert_eq!(lock.navigation(&link, now + at), Navigation::TooSoon);
        }
        assert_eq!(
            lock.navigation(&link, now + OUTSIDE_REPEAT),
            Navigation::OpenOutside(link)
        );
    }

    #[test]
    fn nothing_opens_outside_as_a_page_starts_loading() {
        let (lock, now) = on_server("https://neon.example.com/");
        lock.started(&url("https://neon.example.com/w/acme"), now);
        let away = url("https://example.com/");
        for at in [0, 300, 999] {
            assert_eq!(
                lock.navigation(&away, now + Duration::from_millis(at)),
                Navigation::TooSoon,
                "{at} ms"
            );
        }
        assert_eq!(
            lock.navigation(&away, now + PAGE_SETTLE),
            Navigation::OpenOutside(away)
        );
    }

    #[test]
    fn a_page_of_another_origin_that_no_hook_saw_is_left() {
        let (lock, now) = on_server("https://neon.example.com/w/acme/p/1");
        // A form posted to another origin, which Android loads without asking.
        let posted = url("https://evil.example/collect");
        assert_eq!(
            lock.started(&posted, now),
            Started::Leave {
                outside: Some(posted),
                back: Some(url("https://neon.example.com/w/acme/p/1")),
            }
        );
        // A scheme the lock refuses opens nothing; with the server removed, the way back is
        // the picker.
        lock.forget(&origin(SERVER));
        assert_eq!(
            lock.started(&url("data:text/html,x"), now + Duration::from_secs(5)),
            Started::Leave {
                outside: None,
                back: None
            }
        );
    }

    #[test]
    fn a_page_that_never_starts_is_given_up_on() {
        let (lock, now) = lock();
        let attempt = lock.begin_opening();
        assert!(lock.open_server(origin(SERVER), attempt));
        // The window asked for the server's page, which has not started: the time limit gives
        // up on it, once.
        assert_eq!(lock.stalled(attempt), Some(origin(SERVER)));
        assert_eq!(lock.stalled(attempt), None);
        // The server stays open to the lock: its page may still come, and the way back to the
        // picker takes its place.
        assert_eq!(lock.navigation(&url(SERVER), now), Navigation::Load);
    }

    #[test]
    fn a_page_that_starts_ends_the_wait() {
        let (lock, now) = lock();
        let attempt = lock.begin_opening();
        assert!(lock.open_server(origin(SERVER), attempt));
        lock.started(&url("https://neon.example.com/w/acme"), now);
        assert_eq!(lock.stalled(attempt), None);
        // Back on the picker later, nothing of that attempt is left to cancel.
        lock.started(&url(PICKER), now);
        assert_eq!(lock.cancel_opening(), None);
    }

    #[test]
    fn cancelling_stops_the_page_or_the_check_under_way() {
        let (lock, _) = lock();
        // While the check runs: nothing to stop in the window, and the attempt opens nothing
        // once its check is done.
        let checking = lock.begin_opening();
        assert_eq!(lock.cancel_opening(), None);
        assert!(!lock.still_opening(checking));
        assert!(!lock.open_server(origin(SERVER), checking));
        assert_eq!(
            lock.judge(&url(SERVER)),
            Verdict::OpenOutside,
            "the lock stays closed to it"
        );

        // While the page loads: the server whose page the picker must stop.
        let loading = lock.begin_opening();
        assert!(lock.open_server(origin(SERVER), loading));
        assert_eq!(lock.cancel_opening(), Some(origin(SERVER)));
        assert_eq!(
            lock.stalled(loading),
            None,
            "the time limit has nothing left"
        );
    }

    #[test]
    fn a_newer_attempt_or_the_way_back_ends_the_older_one() {
        let (lock, _) = lock();
        let first = lock.begin_opening();
        let second = lock.begin_opening();
        assert!(!lock.open_server(origin(SERVER), first));
        assert!(lock.open_server(origin(SERVER), second));
        // The menu's way back to the picker, while the page loads.
        lock.show_picker();
        assert!(!lock.still_opening(second));
        assert_eq!(lock.stalled(second), None);
        assert_eq!(lock.cancel_opening(), None);
    }

    #[test]
    fn the_rules_verdicts_carry_through() {
        let (lock, now) = on_server("https://neon.example.com/");
        for (page, expected) in [
            ("https://neon.example.com/w", Navigation::Load),
            ("blob:https://neon.example.com/0b0e2f7a", Navigation::Load),
            ("about:srcdoc", Navigation::Load),
            ("file:///etc/passwd", Navigation::Refuse),
            ("javascript:alert(1)", Navigation::Refuse),
        ] {
            assert_eq!(lock.navigation(&url(page), now), expected, "{page}");
        }
        assert_eq!(
            lock.judge(&url("https://neon.example.com/x")),
            Verdict::Load(Place::Server)
        );
    }
}
