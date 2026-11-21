use futures::{stream::FuturesUnordered, StreamExt};
use rd_interface::{registry::NetRef, Context, Error, IntoAddress, Net};
use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    time::{timeout, Duration, Instant},
};

pub(crate) const DEFAULT_TEST_URL: &str = "http://www.gstatic.com/generate_204";
pub(crate) const DEFAULT_TEST_TIMEOUT_MS: u64 = 5_000;
pub(crate) const DEFAULT_MAX_FAILED_TIMES: u32 = 5;
/// After the first health check succeeds, keep collecting for a short window so
/// a slow dead node cannot hold up the first request for the whole timeout.
const PROBE_RESULT_WINDOW: Duration = Duration::from_millis(800);

pub(crate) fn default_test_url() -> String {
    DEFAULT_TEST_URL.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoSelectMode {
    UrlTest,
    Fallback,
}

#[derive(Debug)]
struct AutoSelectState {
    selected_idx: usize,
    last_checked_at: Option<Instant>,
    failed_times: u32,
    failed_time: Option<Instant>,
}

pub(crate) struct AutoSelectCore {
    mode: AutoSelectMode,
    list: Vec<Net>,
    test_url: Url,
    interval: Duration,
    tolerance: u64,
    test_timeout: Duration,
    max_failed_times: u32,
    state: Mutex<AutoSelectState>,
}

impl AutoSelectCore {
    pub(crate) fn new(
        mode: AutoSelectMode,
        selected: NetRef,
        list: Vec<NetRef>,
        url: String,
        interval: u64,
        tolerance: u64,
        test_timeout: u64,
        max_failed_times: u32,
    ) -> rd_interface::Result<Self> {
        if list.is_empty() {
            return Err(Error::other(format!("{mode:?} list is empty")));
        }

        let selected_idx = list
            .iter()
            .position(|item| item.represent() == selected.represent())
            .unwrap_or(0);
        let nets = list.into_iter().map(|item| item.value_cloned()).collect();
        let test_url = Url::parse(&url)
            .map_err(|err| Error::other(format!("invalid auto-select url {url}: {err}")))?;

        Ok(Self {
            mode,
            list: nets,
            test_url,
            interval: Duration::from_secs(interval),
            tolerance,
            test_timeout: Duration::from_millis(if test_timeout == 0 {
                DEFAULT_TEST_TIMEOUT_MS
            } else {
                test_timeout
            }),
            max_failed_times: if max_failed_times == 0 {
                DEFAULT_MAX_FAILED_TIMES
            } else {
                max_failed_times
            },
            state: Mutex::new(AutoSelectState {
                selected_idx,
                last_checked_at: None,
                failed_times: 0,
                failed_time: None,
            }),
        })
    }

    fn should_refresh(&self, state: &AutoSelectState) -> bool {
        match state.last_checked_at {
            None => true,
            Some(_) if self.interval.is_zero() => false,
            Some(last_checked_at) => last_checked_at.elapsed() >= self.interval,
        }
    }

    pub(crate) async fn current_index(&self) -> rd_interface::Result<usize> {
        let mut state = self.state.lock().await;
        if !self.should_refresh(&state) {
            return Ok(state.selected_idx);
        }

        let current_idx = state.selected_idx.min(self.list.len().saturating_sub(1));
        let next_idx = match self.mode {
            AutoSelectMode::UrlTest => self.pick_url_test_index(current_idx).await?,
            AutoSelectMode::Fallback => self.pick_fallback_index(current_idx).await?,
        };
        state.selected_idx = next_idx;
        state.last_checked_at = Some(Instant::now());
        Ok(next_idx)
    }

    pub(crate) async fn current_net(&self) -> rd_interface::Result<Net> {
        Ok(self.list[self.current_index().await?].clone())
    }

    pub(crate) async fn on_operation_success(&self) {
        let mut state = self.state.lock().await;
        state.failed_times = 0;
        state.failed_time = None;
    }

    pub(crate) async fn on_operation_failure(&self, err: &rd_interface::Error) {
        let mut state = self.state.lock().await;
        let err = err.to_string();
        if err.contains("connection refused") {
            state.last_checked_at = None;
            state.failed_times = 0;
            state.failed_time = None;
            return;
        }

        let now = Instant::now();
        match state.failed_time {
            Some(first_failed_at) if first_failed_at.elapsed() <= self.test_timeout => {
                state.failed_times += 1;
            }
            _ => {
                state.failed_times = 1;
                state.failed_time = Some(now);
            }
        }

        if state.failed_times >= self.max_failed_times {
            state.last_checked_at = None;
            state.failed_times = 0;
            state.failed_time = None;
        }
    }

    async fn pick_fallback_index(&self, current_idx: usize) -> rd_interface::Result<usize> {
        let results = self.collect_probes().await;
        Ok(results
            .into_iter()
            .map(|(idx, _)| idx)
            .min()
            .unwrap_or_else(|| current_idx.min(self.list.len().saturating_sub(1))))
    }

    async fn pick_url_test_index(&self, current_idx: usize) -> rd_interface::Result<usize> {
        let results = self.collect_probes().await;
        let current_delay = results
            .iter()
            .find(|(idx, _)| *idx == current_idx)
            .map(|(_, delay)| *delay);
        let best = results
            .into_iter()
            .min_by_key(|(idx, delay)| (*delay, *idx));

        match best {
            Some((best_idx, best_delay)) => {
                if best_idx == current_idx {
                    return Ok(best_idx);
                }
                if let Some(delay) = current_delay {
                    if delay <= best_delay.saturating_add(self.tolerance) {
                        return Ok(current_idx.min(self.list.len().saturating_sub(1)));
                    }
                }
                Ok(best_idx)
            }
            None => Ok(current_idx.min(self.list.len().saturating_sub(1))),
        }
    }

    async fn collect_probes(&self) -> Vec<(usize, u64)> {
        // Probe every candidate at once: a subscription can contain dozens of
        // nodes, and dead ones take the full timeout.
        let mut probes = FuturesUnordered::new();
        for (idx, net) in self.list.iter().enumerate() {
            probes.push(async move {
                Self::probe_delay(net, &self.test_url, self.test_timeout)
                    .await
                    .ok()
                    .map(|delay| (idx, delay))
            });
        }

        let deadline = Instant::now() + self.test_timeout;
        let mut results = Vec::new();
        let mut result_window: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let deadline = result_window.unwrap_or(deadline).min(deadline);
            let remaining = deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                break;
            }

            match timeout(remaining, probes.next()).await {
                Ok(Some(Some(result))) => {
                    results.push(result);
                    if result_window.is_none() {
                        result_window = Some(now + PROBE_RESULT_WINDOW);
                    }
                }
                Ok(Some(None)) => {}
                Ok(None) => break,
                Err(_) => break,
            }
        }
        results
    }

    async fn probe_delay(
        net: &Net,
        url: &Url,
        test_timeout: Duration,
    ) -> rd_interface::Result<u64> {
        let host = url
            .host_str()
            .ok_or_else(|| Error::other(format!("missing host in probe url: {url}")))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| Error::other(format!("missing port in probe url: {url}")))?;

        timeout(test_timeout, async {
            let start = Instant::now();
            let mut socket = net
                .tcp_connect(&mut Context::new(), &(host, port).into_address()?)
                .await?;

            let host_header = match url.port_or_known_default() {
                Some(p) => format!("{}:{}", host, p),
                None => host.to_string(),
            };
            let mut path_and_query = url.path().to_string();
            if path_and_query.is_empty() {
                path_and_query = "/".to_string();
            }
            if let Some(query) = url.query() {
                path_and_query.push('?');
                path_and_query.push_str(query);
            }
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
                path = path_and_query,
                host = host_header
            );
            socket.write_all(request.as_bytes()).await?;
            socket.flush().await?;

            let mut one = [0u8; 1];
            socket.read_exact(&mut one).await?;
            Ok::<u64, rd_interface::Error>(start.elapsed().as_millis() as u64)
        })
        .await?
    }
}
