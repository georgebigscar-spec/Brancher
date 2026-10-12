//! Сервисы: фоновое выполнение [`Job`] для каждого репозитория и наблюдение за его файлами.
//!
//! Правило потоков (раздел 3): UI-поток не запускает процессы. У каждого репозитория
//! свой worker, задачи изменения выполняются строго по одной (без гонок за `index.lock`).

mod executor;
pub mod highlight;
mod queue;
pub mod session;
mod watch;

pub use executor::execute;
pub use queue::{Completed, Jobs, Repaint};

pub use brancher_core::{Job, JobResult, RepoId};
