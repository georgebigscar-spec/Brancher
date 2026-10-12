//! Сервисы: фоновое выполнение [`Job`] для каждого репозитория.
//!
//! Правило потоков (раздел 3): UI-поток не запускает процессы. У каждого репозитория
//! свой worker, задачи изменения выполняются строго по одной (без гонок за `index.lock`).

mod executor;
mod queue;

pub use executor::execute;
pub use queue::{Completed, Jobs, Repaint};

pub use brancher_core::{Job, JobResult, RepoId};
