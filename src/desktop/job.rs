//! Slow work, kept off the thread that draws the window.
//!
//! Making a delta at 400 dpi takes seconds; reading a page of letters takes
//! about one. Doing either between two frames freezes the window — the title
//! bar greys out, the operating system offers to kill the program, and the
//! person concludes it has crashed. It has not; it is working, and the only
//! thing wrong is that it never said so.
//!
//! So every slow thing runs on a thread of its own and reports back. The window
//! keeps drawing, shows what is happening, and stays honest about it.
//!
//! # One at a time, on purpose
//!
//! pdfium is not safe to use from two threads at once — the library serialises
//! individual calls, not the sequence of calls that makes up one document, so
//! two renders at once will eventually read one another's state and crash. The
//! rest of Onionskin already respects this through `render::engine()`. Here it
//! is respected by structure: there is one worker, so there is one job.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

/// What a job says about itself while it runs.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    /// What is happening, in words a person would use.
    pub doing: String,
    /// How far along, if it is knowable. Not every job can say.
    pub fraction: Option<f32>,
}

impl Progress {
    pub fn saying(doing: impl Into<String>) -> Progress {
        Progress {
            doing: doing.into(),
            fraction: None,
        }
    }

    pub fn part_way(doing: impl Into<String>, fraction: f32) -> Progress {
        Progress {
            doing: doing.into(),
            fraction: Some(fraction.clamp(0.0, 1.0)),
        }
    }
}

/// What a finished job produced.
///
/// Every screen's work ends in one of these, so the shell can report a result
/// or a refusal the same way wherever it came from.
#[derive(Debug)]
pub enum Outcome {
    /// It worked. The message is what to tell the person, and the paths are
    /// whatever was written, so the shell can offer to open the folder.
    Done {
        message: String,
        wrote: Vec<std::path::PathBuf>,
        /// Anything worth saying that is not a failure — a missing calibration
        /// profile, an addition close to the paper's edge.
        notes: Vec<String>,
    },
    /// It refused, and said why. Not an error in the program: a refusal is
    /// often the most useful thing Onionskin does.
    Refused { message: String },
}

impl Outcome {
    pub fn done(message: impl Into<String>) -> Outcome {
        Outcome::Done {
            message: message.into(),
            wrote: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn wrote(message: impl Into<String>, wrote: Vec<std::path::PathBuf>) -> Outcome {
        Outcome::Done {
            message: message.into(),
            wrote,
            notes: Vec::new(),
        }
    }

    pub fn refused(message: impl Into<String>) -> Outcome {
        Outcome::Refused {
            message: message.into(),
        }
    }
}

/// The handle a job body uses to say what it is doing.
#[derive(Clone)]
pub struct Reporter {
    to_window: Sender<Message>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Reporter {
    /// Say what is happening now. Cheap, and safe to call often.
    pub fn saying(&self, doing: impl Into<String>) {
        self.send(Progress::saying(doing));
    }

    pub fn part_way(&self, doing: impl Into<String>, fraction: f32) {
        self.send(Progress::part_way(doing, fraction));
    }

    fn send(&self, progress: Progress) {
        // A closed channel means the window has gone. Nothing to be done about
        // it and nothing worth saying, so the job simply carries on and ends.
        let _ = self.to_window.send(Message::Progress(progress));
        (self.repaint)();
    }
}

enum Message {
    Progress(Progress),
    Finished(Box<Outcome>),
}

/// The one worker, and what it is doing.
pub struct Jobs {
    from_worker: Option<Receiver<Message>>,
    running: Option<Running>,
    /// The last thing that finished, until it is dismissed.
    pub last: Option<Outcome>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

struct Running {
    what: String,
    progress: Progress,
    started: std::time::Instant,
}

impl Jobs {
    pub fn new(ctx: &eframe::egui::Context) -> Jobs {
        // Waking the window is how a background thread gets anything drawn: a
        // window that only redraws on input would show the same "working…" for
        // as long as the work took, and change nothing when it finished.
        let ctx = ctx.clone();
        Jobs {
            from_worker: None,
            running: None,
            last: None,
            repaint: Arc::new(move || ctx.request_repaint()),
        }
    }

    /// Is something running? Screens use this to grey out their buttons rather
    /// than letting a second job be started that would have to queue anyway.
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// What is happening, for the status bar.
    pub fn doing(&self) -> Option<(&str, &Progress, std::time::Duration)> {
        self.running
            .as_ref()
            .map(|r| (r.what.as_str(), &r.progress, r.started.elapsed()))
    }

    /// Start something. Refused, quietly, if a job is already running — the
    /// caller should have greyed its button out.
    pub fn start<F>(&mut self, what: impl Into<String>, body: F)
    where
        F: FnOnce(&Reporter) -> Outcome + Send + 'static,
    {
        if self.busy() {
            return;
        }
        let what = what.into();
        let (to_window, from_worker) = std::sync::mpsc::channel();
        let reporter = Reporter {
            to_window: to_window.clone(),
            repaint: self.repaint.clone(),
        };
        let repaint = self.repaint.clone();

        std::thread::Builder::new()
            .name("onionskin-work".into())
            .spawn(move || {
                // A panic in the work must not take the window with it. The
                // person gets told something went wrong and can try again,
                // which is far better than the window vanishing.
                let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    body(&reporter)
                })) {
                    Ok(outcome) => outcome,
                    Err(_) => Outcome::refused(
                        "Something went wrong inside Onionskin, and it stopped rather \
                             than carry on with a job it could not finish.\n\nNothing was \
                             written. If it happens again with the same files, that is a \
                             bug worth reporting.",
                    ),
                };
                let _ = to_window.send(Message::Finished(Box::new(outcome)));
                repaint();
            })
            .expect("a thread to run the work on");

        self.running = Some(Running {
            what: what.clone(),
            progress: Progress::saying("Starting…"),
            started: std::time::Instant::now(),
        });
        self.from_worker = Some(from_worker);
        self.last = None;
    }

    /// Take in whatever the worker has said. Called once a frame.
    pub fn poll(&mut self) {
        let Some(channel) = &self.from_worker else {
            return;
        };
        loop {
            match channel.try_recv() {
                Ok(Message::Progress(progress)) => {
                    if let Some(running) = &mut self.running {
                        running.progress = progress;
                    }
                }
                Ok(Message::Finished(outcome)) => {
                    self.last = Some(*outcome);
                    self.running = None;
                    self.from_worker = None;
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread went without sending an outcome, which should
                    // not happen — but leaving the window saying "working…"
                    // for ever is the one response that helps nobody.
                    if self.running.is_some() {
                        self.last = Some(Outcome::refused(
                            "The work stopped without saying why. Nothing was written.",
                        ));
                    }
                    self.running = None;
                    self.from_worker = None;
                    return;
                }
            }
        }
    }

    /// Forget the last result, when the person has read it.
    pub fn dismiss(&mut self) {
        self.last = None;
    }
}
