//! Drives the running app from a script of steps and saves screenshots of its windows as the
//! window server composites them, native glass included, so a UI change can be looked at
//! without opening the app by hand. An app may capture its own windows without the Screen
//! Recording permission.
//!
//! ```sh
//! FLINT_UI_SCRIPT=script/ui-scripts/surfaces.json FLINT_UI_SCRIPT_OUTPUT=target/ui-script \
//!     flint --user-data-dir target/ui-script/data path/to/project
//! ```
//!
//! The app quits when the script ends; a failed step is logged and written to `error.txt` in the
//! output directory.

use anyhow::{Context as _, Result};
use collections::HashMap;
use gpui::{
    AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PlatformInput, point, px,
};
use hdrhistogram::Histogram;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use workspace::{MultiWorkspace, OpenOptions};

const SCRIPT_VARIABLE: &str = "FLINT_UI_SCRIPT";
const OUTPUT_VARIABLE: &str = "FLINT_UI_SCRIPT_OUTPUT";
const DEFAULT_OUTPUT: &str = "target/ui-script";
const WINDOW_TIMEOUT: Duration = Duration::from_secs(60);
/// The app draws while a script runs as a visible window would, even when the window is
/// covered and has stopped its own display link: input reaches the elements of the last drawn
/// frame, so a menu opened by one step only takes the next step's keys once it was drawn.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Frames drawn after each input step, so what it changed is drawn before the next one.
const FRAMES_AFTER_INPUT: u32 = 3;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    /// Waits, so scans, language servers and animations settle.
    Wait {
        milliseconds: u64,
    },
    /// Dispatches an action by name to the focused element, as a key binding would.
    Action {
        name: String,
        #[serde(default)]
        arguments: Option<serde_json::Value>,
        /// Frames drawn afterwards; 3 by default. With 0, a following `capture` with `frames: 1`
        /// sees the very first frame the input produced.
        #[serde(default)]
        frames: Option<u32>,
    },
    /// Types keystrokes such as `cmd-shift-p`, separated by spaces.
    Keystrokes {
        keys: String,
        #[serde(default)]
        frames: Option<u32>,
    },
    /// Opens a file in the workspace; a relative path starts at the first project folder.
    Open {
        path: PathBuf,
    },
    /// Clicks at a position in logical pixels from the window's top-left corner.
    Click {
        x: f32,
        y: f32,
        #[serde(default)]
        right: bool,
        #[serde(default)]
        frames: Option<u32>,
    },
    /// Moves the mouse to a position, so hovers and tooltips appear.
    Hover {
        x: f32,
        y: f32,
        #[serde(default)]
        frames: Option<u32>,
    },
    /// Saves every window of the app as `<name>.png`, or `<name>-<n>.png` when there are several,
    /// after drawing `frames` frames (3 by default). Consecutive captures with `frames: 1` record
    /// a transition frame by frame, which shows flicker.
    Capture {
        name: String,
        #[serde(default)]
        frames: Option<u32>,
    },
    /// Runs steps repeatedly, for leak and performance checks.
    Repeat {
        times: usize,
        steps: Vec<Step>,
    },
    /// Logs the process's physical memory footprint, as `footprint` and Activity Monitor report it.
    Memory {
        label: String,
    },
    /// Logs how long the active window's frames took to draw since the previous `frame_times`
    /// step for that window, or since it opened.
    FrameTimes {
        label: String,
    },
    Quit,
}

pub fn run_if_requested(cx: &mut App) {
    let Ok(script_path) = std::env::var(SCRIPT_VARIABLE) else {
        return;
    };
    let output = std::env::var(OUTPUT_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_OUTPUT));
    cx.spawn(async move |cx| {
        // macOS naps an app whose windows are covered: it moves it to slower cores and stretches
        // its timers, so frame times would measure the throttling rather than the app.
        let _app_nap_guard = cx
            .background_executor()
            .prevent_app_nap("running a UI script");
        let started = Instant::now();
        let result = run_script(Path::new(&script_path), &output, cx).await;
        match &result {
            Ok(()) => log::info!("UI script finished in {:?}", started.elapsed()),
            Err(error) => {
                log::error!("UI script failed: {error:#}");
                if let Err(write_error) =
                    std::fs::write(output.join("error.txt"), format!("{error:#}\n"))
                {
                    log::error!("could not write the UI script error: {write_error}");
                }
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn run_script(script_path: &Path, output: &Path, cx: &mut AsyncApp) -> Result<()> {
    let script: Script = serde_json::from_str(
        &std::fs::read_to_string(script_path)
            .with_context(|| format!("reading the UI script {}", script_path.display()))?,
    )
    .with_context(|| format!("parsing the UI script {}", script_path.display()))?;
    std::fs::create_dir_all(output)
        .with_context(|| format!("creating the output directory {}", output.display()))?;
    wait_for_workspace_window(cx).await?;
    let mut run = Run {
        output: output.to_path_buf(),
        reported_frames: HashMap::default(),
    };
    run_steps(&script.steps, &mut run, cx).await
}

struct Run {
    output: PathBuf,
    reported_frames: HashMap<AnyWindowHandle, Histogram<u64>>,
}

async fn wait_for_workspace_window(cx: &mut AsyncApp) -> Result<()> {
    let deadline = Instant::now() + WINDOW_TIMEOUT;
    loop {
        let has_workspace = cx.update(|cx| {
            cx.windows()
                .iter()
                .any(|window| window.downcast::<MultiWorkspace>().is_some())
        });
        if has_workspace {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "no workspace window opened within {WINDOW_TIMEOUT:?}"
        );
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
}

async fn run_steps(steps: &[Step], run: &mut Run, cx: &mut AsyncApp) -> Result<()> {
    for step in steps {
        log::info!("UI script step: {step:?}");
        run_step(step, run, cx)
            .await
            .with_context(|| format!("running {step:?}"))?;
    }
    Ok(())
}

async fn run_step(step: &Step, run: &mut Run, cx: &mut AsyncApp) -> Result<()> {
    match step {
        Step::Wait { milliseconds } => {
            draw_frames_for(Duration::from_millis(*milliseconds), cx).await?;
        }
        Step::Action {
            name,
            arguments,
            frames,
        } => {
            active_window(cx)?.update(cx, |_, window, cx| -> Result<()> {
                let action = cx.build_action(name, arguments.clone())?;
                window.dispatch_action(action, cx);
                Ok(())
            })??;
            draw_frames_after_input(frames.unwrap_or(FRAMES_AFTER_INPUT), cx).await?;
        }
        Step::Keystrokes { keys, frames } => {
            let keystrokes = keys
                .split_whitespace()
                .map(Keystroke::parse)
                .collect::<Result<Vec<_>, _>>()?;
            for keystroke in keystrokes {
                active_window(cx)?
                    .update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx))?;
                draw_frames_after_input(frames.unwrap_or(FRAMES_AFTER_INPUT), cx).await?;
            }
        }
        Step::Open { path } => {
            let window = active_window(cx)?
                .downcast::<MultiWorkspace>()
                .context("the active window is not a workspace")?;
            let open = window.update(cx, |multi_workspace, window, cx| {
                multi_workspace.workspace().update(cx, |workspace, cx| {
                    let path = if path.is_relative() {
                        let project_folder = workspace
                            .project()
                            .read(cx)
                            .visible_worktrees(cx)
                            .next()
                            .map(|worktree| worktree.read(cx).abs_path())
                            .context("the workspace has no project folder")?;
                        project_folder.join(path)
                    } else {
                        path.clone()
                    };
                    anyhow::Ok(workspace.open_abs_path(path, OpenOptions::default(), window, cx))
                })
            })??;
            open.await?;
            draw_frames_after_input(FRAMES_AFTER_INPUT, cx).await?;
        }
        Step::Click {
            x,
            y,
            right,
            frames,
        } => {
            let position = point(px(*x), px(*y));
            let button = if *right {
                MouseButton::Right
            } else {
                MouseButton::Left
            };
            active_window(cx)?.update(cx, |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent {
                        button,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }),
                    cx,
                );
            })?;
            draw_frames_after_input(frames.unwrap_or(FRAMES_AFTER_INPUT), cx).await?;
        }
        Step::Hover { x, y, frames } => {
            active_window(cx)?.update(cx, |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position: point(px(*x), px(*y)),
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
            })?;
            draw_frames_after_input(frames.unwrap_or(FRAMES_AFTER_INPUT), cx).await?;
        }
        Step::Capture { name, frames } => {
            capture_windows(name, frames.unwrap_or(FRAMES_AFTER_INPUT), &run.output, cx).await?
        }
        Step::Repeat { times, steps } => {
            for _ in 0..*times {
                Box::pin(run_steps(steps, run, cx)).await?;
            }
        }
        Step::Memory { label } => {
            log::info!(
                "UI script memory {label}: {} MB physical footprint",
                physical_footprint()? / (1024 * 1024)
            );
        }
        Step::FrameTimes { label } => {
            let window = active_window(cx)?;
            let frames = window.update(cx, |_, window, _| window.frame_duration_snapshot())?;
            let mut draw = frames.draw_duration_histogram.clone();
            if let Some(reported) = run
                .reported_frames
                .insert(window, frames.draw_duration_histogram)
            {
                draw.subtract(reported)
                    .context("removing the frames reported before")?;
            }
            let milliseconds = |nanoseconds: u64| nanoseconds as f64 / 1_000_000.;
            log::info!(
                "UI script frame times {label}: {} frames drawn, median {:.2} ms, 95th percentile {:.2} ms, 99th {:.2} ms, slowest {:.2} ms",
                draw.len(),
                milliseconds(draw.value_at_quantile(0.5)),
                milliseconds(draw.value_at_quantile(0.95)),
                milliseconds(draw.value_at_quantile(0.99)),
                milliseconds(draw.max()),
            );
        }
        Step::Quit => cx.update(|cx| cx.quit()),
    }
    Ok(())
}

fn active_window(cx: &mut AsyncApp) -> Result<AnyWindowHandle> {
    cx.update(|cx| {
        cx.active_window()
            .or_else(|| cx.windows().into_iter().next())
    })
    .context("the app has no window")
}

async fn draw_frames_for(duration: Duration, cx: &mut AsyncApp) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        draw_frame(cx)?;
        if Instant::now() >= deadline {
            return Ok(());
        }
        cx.background_executor().timer(FRAME_INTERVAL).await;
    }
}

async fn draw_frames_after_input(frames: u32, cx: &mut AsyncApp) -> Result<()> {
    for _ in 0..frames {
        cx.background_executor().timer(FRAME_INTERVAL).await;
        draw_frame(cx)?;
    }
    Ok(())
}

fn draw_frame(cx: &mut AsyncApp) -> Result<()> {
    for window in cx.update(|cx| cx.windows()) {
        window.update(cx, |_, window, cx| window.draw_and_present(cx))?;
    }
    Ok(())
}

async fn capture_windows(name: &str, frames: u32, output: &Path, cx: &mut AsyncApp) -> Result<()> {
    draw_frames_after_input(frames, cx).await?;
    // The last frame reaches the window server when Core Animation commits it at the end of this
    // run loop pass, so the capture waits for the next one.
    cx.background_executor().timer(FRAME_INTERVAL).await;
    let windows = cx.update(|cx| cx.windows());
    for (index, window) in windows.iter().enumerate() {
        let image = window.update(cx, |_, window, _| window.capture_composited_image())??;
        let file_name = if windows.len() == 1 {
            format!("{name}.png")
        } else {
            format!("{name}-{}.png", index + 1)
        };
        let path = output.join(file_name);
        image
            .save(&path)
            .with_context(|| format!("saving {}", path.display()))?;
        log::info!("UI script captured {}", path.display());
    }
    Ok(())
}

fn physical_footprint() -> Result<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `proc_pid_rusage` fills a `rusage_info_v2` for the `RUSAGE_INFO_V2` flavor.
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr().cast(),
        )
    };
    anyhow::ensure!(status == 0, "proc_pid_rusage failed with {status}");
    // SAFETY: the call succeeded, so the struct is initialized.
    Ok(unsafe { usage.assume_init() }.ri_phys_footprint)
}
