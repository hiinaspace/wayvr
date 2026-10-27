//! Swipe recognizers behind one interface, so the same captured swipe can be
//! decoded by CleverKeys (super-swipe-type, in-process) or FUTO Swipe
//! (swipesrv, a local HTTP sidecar), and both answers logged for comparison.
//!
//! Environment:
//! - `WAYVR_SWIPE_ENGINE`: `cleverkeys` (default) or `futo`; picks the
//!   suggestions shown on the keyboard.
//! - `WAYVR_SWIPE_FUTO_URL`: swipesrv endpoint, default
//!   `http://127.0.0.1:8012/recognize`. Plain http only.
//! - `WAYVR_SWIPE_LOG`: if set, run every available engine on each swipe and
//!   append one JSON line per swipe (and per selected word) to this path.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use glam::Vec2;
use super_swipe_type::SwipePoint;
use super_swipe_type::keyboard_manager::QwertyKeyboardGrid;
use super_swipe_type::swipe_orchestrator::SwipeOrchestrator;

/// One captured sample: which letter key the pointer was over, where inside
/// that key (0..1, y down), and time since the swipe started.
#[derive(Clone, Copy, Debug)]
pub struct RawSwipePoint {
    pub key: char,
    pub within_key: Vec2,
    pub elapsed: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineKind {
    CleverKeys,
    Futo,
}

impl EngineKind {
    pub fn from_env() -> Self {
        match std::env::var("WAYVR_SWIPE_ENGINE").as_deref() {
            Ok("futo") => Self::Futo,
            _ => Self::CleverKeys,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::CleverKeys => "cleverkeys",
            Self::Futo => "futo",
        }
    }
}

pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("WAYVR_SWIPE_LOG").map(PathBuf::from)
}

/// Lives on the prediction worker thread.
pub struct SwipeEngines {
    primary: EngineKind,
    cleverkeys: Option<CleverKeys>,
    futo: Option<FutoClient>,
    log: Option<PathBuf>,
}

impl SwipeEngines {
    pub fn new(cleverkeys_model: &Path) -> Self {
        let primary = EngineKind::from_env();
        let log = log_path();
        let want_all = log.is_some();

        let cleverkeys = (primary == EngineKind::CleverKeys || want_all)
            .then(
                || match SwipeOrchestrator::new_from_path(cleverkeys_model) {
                    Ok(orchestrator) => Some(CleverKeys {
                        orchestrator,
                        grid: QwertyKeyboardGrid::new(),
                    }),
                    Err(e) => {
                        log::error!("Failed to initialize SwipeOrchestrator: {e}");
                        None
                    }
                },
            )
            .flatten();
        let futo = (primary == EngineKind::Futo || want_all).then(FutoClient::from_env);

        log::info!(
            "Swipe engines: primary={}, cleverkeys={}, futo={}, log={:?}",
            primary.name(),
            cleverkeys.is_some(),
            futo.as_ref().map_or("off", |f| f.url.as_str()),
            log
        );
        Self {
            primary,
            cleverkeys,
            futo,
            log,
        }
    }

    /// Runs the primary engine and hands its words (best first) to `publish`
    /// right away. Only then, if logging, runs the other engine and writes
    /// the comparison line, so the slower engine never delays suggestions.
    pub fn predict(
        &mut self,
        swipe: &[RawSwipePoint],
        last_word: &Option<String>,
        publish: impl FnOnce(anyhow::Result<Vec<String>>),
    ) {
        let primary = self.run(self.primary, swipe, last_word);
        let primary_json = primary.as_ref().map(result_json);
        match primary {
            Some((result, _)) => publish(result),
            None => publish(Err(anyhow::anyhow!(
                "{} swipe engine unavailable",
                self.primary.name()
            ))),
        }

        let Some(path) = self.log.clone() else {
            return;
        };
        let other = match self.primary {
            EngineKind::CleverKeys => EngineKind::Futo,
            EngineKind::Futo => EngineKind::CleverKeys,
        };
        let other_json = self.run(other, swipe, last_word).as_ref().map(result_json);
        let (clever_json, futo_json) = match self.primary {
            EngineKind::CleverKeys => (primary_json, other_json),
            EngineKind::Futo => (other_json, primary_json),
        };
        let points: Vec<_> = swipe
            .iter()
            .map(|p| {
                serde_json::json!([
                    p.key.to_string(),
                    p.within_key.x,
                    p.within_key.y,
                    p.elapsed.as_secs_f64() * 1000.0
                ])
            })
            .collect();
        append_log(
            &path,
            &serde_json::json!({
                "type": "swipe",
                "at": unix_ms(),
                "primary": self.primary.name(),
                "last_word": last_word,
                "points": points,
                "cleverkeys": clever_json,
                "futo": futo_json,
            }),
        );
    }

    fn run(
        &mut self,
        kind: EngineKind,
        swipe: &[RawSwipePoint],
        last_word: &Option<String>,
    ) -> Option<(anyhow::Result<Vec<String>>, Duration)> {
        let start = Instant::now();
        let result = match kind {
            EngineKind::CleverKeys => self.cleverkeys.as_mut()?.predict(swipe, last_word),
            EngineKind::Futo => self.futo.as_ref()?.predict(swipe, last_word),
        };
        Some((result, start.elapsed()))
    }
}

fn result_json((result, elapsed): &(anyhow::Result<Vec<String>>, Duration)) -> serde_json::Value {
    match result {
        Ok(words) => serde_json::json!({ "words": words, "ms": elapsed.as_secs_f64() * 1000.0 }),
        Err(e) => serde_json::json!({ "error": format!("{e:#}") }),
    }
}

/// Records which suggestion the user actually committed, for scoring logs.
pub fn log_selection(word: &str) {
    if let Some(path) = log_path() {
        append_log(
            &path,
            &serde_json::json!({ "type": "select", "at": unix_ms(), "word": word }),
        );
    }
}

fn append_log(path: &Path, value: &serde_json::Value) {
    let res = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| writeln!(f, "{value}"));
    if let Err(e) = res {
        log::warn!("Could not write swipe log {}: {e}", path.display());
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

struct CleverKeys {
    orchestrator: SwipeOrchestrator,
    grid: QwertyKeyboardGrid,
}

impl CleverKeys {
    fn predict(
        &mut self,
        swipe: &[RawSwipePoint],
        last_word: &Option<String>,
    ) -> anyhow::Result<Vec<String>> {
        let key_dimensions = Vec2::new(
            QwertyKeyboardGrid::get_key_width() as f32,
            QwertyKeyboardGrid::get_key_height() as f32,
        );
        let points = swipe
            .iter()
            .filter_map(|p| {
                let pos = self.grid.key_positions.get(&p.key)?;
                let key_pos = Vec2::new(pos.x as f32, pos.y as f32);
                let point = (p.within_key - Vec2::splat(0.5)) * key_dimensions + key_pos;
                // multiply by .8 because library is trained on mobile swipes which happen on a smaller keyboard and are faster
                Some(SwipePoint::new(
                    point.x.into(),
                    point.y.into(),
                    p.elapsed.mul_f32(0.8),
                ))
            })
            .collect();
        Ok(self
            .orchestrator
            .predict(points, last_word)?
            .into_iter()
            .map(|c| c.word)
            .collect())
    }
}

/// Key centres of swipe-library's `default_en_qwerty()` / `en_qwerty.json`,
/// the layout the FUTO English decoder was trained against. Keys are 0.1 wide
/// and 1/3 tall in this unit square.
fn futo_key_center(key: char) -> Option<Vec2> {
    const ROWS: [(&str, f32, f32); 3] = [
        ("qwertyuiop", 0.05, 0.167),
        ("asdfghjkl", 0.10, 0.500),
        ("zxcvbnm", 0.20, 0.833),
    ];
    ROWS.iter().find_map(|(letters, x0, y)| {
        letters
            .find(key)
            .map(|i| Vec2::new((i as f32).mul_add(0.1, *x0), *y))
    })
}

const FUTO_KEY_SIZE: Vec2 = Vec2::new(0.1, 1.0 / 3.0);

struct FutoClient {
    url: String,
    host_port: String,
    path: String,
}

impl FutoClient {
    fn from_env() -> Self {
        let url = std::env::var("WAYVR_SWIPE_FUTO_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8012/recognize".to_owned());
        let rest = url.strip_prefix("http://").unwrap_or(&url);
        let (host_port, path) = rest
            .split_once('/')
            .map_or((rest.to_owned(), "/".to_owned()), |(h, p)| {
                (h.to_owned(), format!("/{p}"))
            });
        Self {
            url,
            host_port,
            path,
        }
    }

    /// swipesrv body: first line is space-separated context words, then one
    /// `[x; y]t_ms` line per point, coordinates normalized to the layout.
    fn request_body(swipe: &[RawSwipePoint], last_word: &Option<String>) -> String {
        let mut body = last_word.clone().unwrap_or_default();
        body.push('\n');
        for p in swipe {
            let Some(center) = futo_key_center(p.key) else {
                continue;
            };
            let pt = (p.within_key - Vec2::splat(0.5)) * FUTO_KEY_SIZE + center;
            body.push_str(&format!(
                "[{:.4}; {:.4}]{:.1}\n",
                pt.x,
                pt.y,
                p.elapsed.as_secs_f64() * 1000.0
            ));
        }
        body
    }

    fn predict(
        &self,
        swipe: &[RawSwipePoint],
        last_word: &Option<String>,
    ) -> anyhow::Result<Vec<String>> {
        let body = Self::request_body(swipe, last_word);
        let mut stream = TcpStream::connect(&self.host_port)
            .with_context(|| format!("connect to swipesrv at {}", self.url))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        write!(
            stream,
            "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
            self.path,
            self.host_port,
            body.len(),
            body
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let (head, payload) = response
            .split_once("\r\n\r\n")
            .context("malformed swipesrv response")?;
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            bail!(
                "swipesrv error: {} {}",
                head.lines().next().unwrap_or_default(),
                payload.trim()
            );
        }
        Ok(payload
            .trim()
            .split(',')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn futo_centers_match_en_qwerty() {
        for (key, x, y) in [('q', 0.05, 0.167), ('l', 0.9, 0.5), ('m', 0.8, 0.833)] {
            let c = futo_key_center(key).unwrap();
            assert!(c.abs_diff_eq(Vec2::new(x, y), 1e-5), "{key}: {c}");
        }
        assert_eq!(futo_key_center('1'), None);
    }

    #[test]
    fn futo_key_center_tap_maps_to_center() {
        let body = FutoClient::request_body(
            &[RawSwipePoint {
                key: 'q',
                within_key: Vec2::splat(0.5),
                elapsed: Duration::ZERO,
            }],
            &Some("hello".into()),
        );
        assert_eq!(body, "hello\n[0.0500; 0.1670]0.0\n");
    }

    /// Straight-line swipe through key centres on the FUTO grid, sampled the
    /// way the keyboard reports it: the key under the pointer plus the offset
    /// inside it. Gaps between rows/keys are not modelled.
    fn synthetic_swipe(word: &str) -> Vec<RawSwipePoint> {
        let centers: Vec<Vec2> = word.chars().filter_map(futo_key_center).collect();
        let mut out = Vec::new();
        let mut t = Duration::ZERO;
        for pair in centers.windows(2) {
            for i in 0..8 {
                let p = pair[0].lerp(pair[1], i as f32 / 8.0);
                let (key, center) = ('a'..='z')
                    .filter_map(|c| futo_key_center(c).map(|k| (c, k)))
                    .min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p)))
                    .unwrap();
                out.push(RawSwipePoint {
                    key,
                    within_key: (p - center) / FUTO_KEY_SIZE + Vec2::splat(0.5),
                    elapsed: t,
                });
                t += Duration::from_millis(16);
            }
        }
        out.push(RawSwipePoint {
            key: word.chars().last().unwrap(),
            within_key: Vec2::splat(0.5),
            elapsed: t,
        });
        out
    }

    /// Needs a running swipesrv (`WAYVR_SWIPE_FUTO_URL`) and the CleverKeys
    /// archive at `WAYVR_SWIPE_CLEVERKEYS_MODEL`:
    /// `cargo test -p wayvr --bin wayvr swipe_engine -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn compare_engines_on_synthetic_swipes() {
        let model = std::env::var("WAYVR_SWIPE_CLEVERKEYS_MODEL").expect("model path");
        let futo = FutoClient::from_env();
        let mut ck = CleverKeys {
            orchestrator: SwipeOrchestrator::new_from_path(&model).unwrap(),
            grid: QwertyKeyboardGrid::new(),
        };
        let words = [
            "hello", "world", "type", "swipe", "keyboard", "the", "quick", "brown", "virtual",
            "reality", "overlay", "people", "should", "because", "minute",
        ];
        let rank = |list: &[String], w: &str| list.iter().position(|x| x == w);
        let (mut f_top4, mut c_top4) = (0, 0);
        for w in words {
            let swipe = synthetic_swipe(w);
            let f = futo.predict(&swipe, &None).unwrap();
            let c = ck.predict(&swipe, &None).unwrap();
            f_top4 += usize::from(rank(&f, w).is_some_and(|r| r < 4));
            c_top4 += usize::from(rank(&c, w).is_some_and(|r| r < 4));
            println!(
                "{w:>10}  futo {:?}  cleverkeys {:?}",
                &f[..f.len().min(4)],
                &c[..c.len().min(4)]
            );
        }
        println!(
            "top-4: futo {f_top4}/{n}, cleverkeys {c_top4}/{n}",
            n = words.len()
        );
        assert!(f_top4 > 0, "FUTO path never decoded the target word");
    }
}
