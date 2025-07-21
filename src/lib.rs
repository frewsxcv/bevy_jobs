#![warn(
    clippy::unwrap_used,
    clippy::cast_lossless,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::expect_used
)]

use std::{any, future};

pub struct Plugin;

impl bevy_app::Plugin for Plugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_systems(bevy_app::Update, check_system)
            .insert_resource(JobOutcomePayloads(vec![]));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobType {
    Compute,
    Io,
    #[cfg(feature = "tokio")]
    Tokio,
}

#[cfg(not(target_arch = "wasm32"))]
pub trait AsyncReturn<Output>: future::Future<Output = Output> + Send {}
#[cfg(target_arch = "wasm32")]
pub trait AsyncReturn<Output>: future::Future<Output = Output> {}

#[cfg(not(target_arch = "wasm32"))]
impl<F, Output> AsyncReturn<Output> for F where F: future::Future<Output = Output> + Send {}
#[cfg(target_arch = "wasm32")]
impl<F, Output> AsyncReturn<Output> for F where F: future::Future<Output = Output> {}

pub trait Job: any::Any + Sized + Send + Sync {
    type Outcome: any::Any + Send + Sync;

    const JOB_TYPE: JobType = JobType::Compute;

    fn name(&self) -> String;

    fn perform(self, context: Context) -> impl AsyncReturn<Self::Outcome>;

    fn spawn(self, commands: &mut bevy_ecs::system::Commands) -> bevy_ecs::entity::Entity {
        let (outcome_tx, outcome_recv) = async_channel::unbounded::<JobOutcomePayload>();
        let (progress_tx, progress_recv) = async_channel::unbounded::<Progress>();

        let job_name = self.name();
        let in_progress_job = InProgressJob {
            name: job_name.clone(),
            progress: 0,
            progress_recv,
            outcome_recv,
        };

        let task = async move {
            let instant = web_time::Instant::now();
            bevy_log::info!("Starting job '{}'", job_name);
            let outcome = self.perform(Context { progress_tx }).await;
            bevy_log::info!("Completed job '{}' in {:?}", job_name, instant.elapsed());
            if let Err(e) = outcome_tx
                .send(JobOutcomePayload {
                    job_outcome_type_id: any::TypeId::of::<Self>(),
                    job_outcome: Box::new(outcome),
                })
                .await
            {
                bevy_log::error!(
                    "Failed to send result from job {} back to main thread: {:?}",
                    job_name,
                    e
                );
            }
        };

        match Self::JOB_TYPE {
            JobType::Compute => bevy_tasks::AsyncComputeTaskPool::get().spawn(task).detach(),
            JobType::Io => bevy_tasks::IoTaskPool::get().spawn(task).detach(),
            #[cfg(feature = "tokio")]
            JobType::Tokio => spawn_tokio_task(task),
        }

        commands.spawn(in_progress_job).id()
    }
}

fn check_system(
    mut query: bevy_ecs::system::Query<(&mut InProgressJob, bevy_ecs::entity::Entity)>,
    mut commands: bevy_ecs::system::Commands,
    mut finished_jobs: FinishedJobs,
) {
    query.iter_mut().for_each(|(mut in_progress_job, entity)| {
        // TODO: Maybe don't run the `try_recv` below every frame?
        while let Ok(progress) = in_progress_job.progress_recv.try_recv() {
            in_progress_job.progress = progress;
        }

        while let Ok(outcome) = in_progress_job.outcome_recv.try_recv() {
            bevy_log::info!("Job finished");
            commands.entity(entity).despawn();
            finished_jobs.outcomes.0.push(outcome);
        }
    })
}

pub struct Context {
    progress_tx: async_channel::Sender<Progress>,
}

impl Context {
    pub fn send_progress(&self, progress: Progress) -> async_channel::Send<u8> {
        self.progress_tx.send(progress)
    }
}

struct JobOutcomePayload {
    job_outcome_type_id: any::TypeId,
    job_outcome: Box<dyn any::Any + Send + Sync>,
}

#[derive(bevy_ecs::system::SystemParam)]
pub struct JobSpawner<'w, 's> {
    commands: bevy_ecs::system::Commands<'w, 's>,
}

impl JobSpawner<'_, '_> {
    pub fn spawn<J: Job>(&mut self, job: J) -> bevy_ecs::entity::Entity {
        job.spawn(&mut self.commands)
    }
}

type Progress = u8;
pub type ProgressSender = async_channel::Sender<Progress>;

#[derive(bevy_ecs::component::Component)]
pub struct InProgressJob {
    pub name: String,
    pub progress: Progress,
    progress_recv: async_channel::Receiver<Progress>,
    outcome_recv: async_channel::Receiver<JobOutcomePayload>,
}

#[derive(bevy_ecs::system::SystemParam)]
pub struct FinishedJobs<'w, 's> {
    outcomes: bevy_ecs::system::ResMut<'w, JobOutcomePayloads>,
    phantom_data: std::marker::PhantomData<&'s ()>,
}

#[derive(bevy_ecs::prelude::Resource)]
pub struct JobOutcomePayloads(Vec<JobOutcomePayload>);

impl FinishedJobs<'_, '_> {
    #[inline]
    pub fn take_next<J: Job>(&mut self) -> Option<J::Outcome> {
        let index = self
            .outcomes
            .0
            .iter_mut()
            .enumerate()
            .filter(|(_i, outcome_payload)| {
                any::TypeId::of::<J>() == outcome_payload.job_outcome_type_id
                    && outcome_payload.job_outcome.is::<J::Outcome>()
            })
            .map(|(i, _)| i)
            .next()?;
        let outcome_payload = self.outcomes.0.remove(index);
        let outcome = outcome_payload.job_outcome.downcast::<J::Outcome>();
        if outcome.is_err() {
            bevy_log::error!("encountered unexpected job result type");
        }
        outcome.map(|n| *n).ok()
    }
}

#[cfg(feature = "tokio")]
fn spawn_tokio_task<Output: Send + 'static>(
    future: impl future::Future<Output = Output> + Send + 'static,
) {
    {
        static TOKIO_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> =
            std::sync::OnceLock::new();
        let rt = TOKIO_RUNTIME.get_or_init(|| {
            #[cfg(not(target_arch = "wasm32"))]
            let mut runtime = tokio::runtime::Builder::new_multi_thread();
            #[cfg(target_arch = "wasm32")]
            let mut runtime = tokio::runtime::Builder::new_current_thread();
            runtime
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime for background tasks")
        });

        let _ = rt.spawn(future);
    }
}

#[cfg(test)]
mod test {
    fn readme() {
        type Error = ();
        static URL: &str = "https://example.com";
        fn fetch(_url: &str) -> impl std::future::Future<Output = Result<Vec<u8>, Error>> {
            async move {
                // Simulate fetching data
                Ok(vec![1, 2, 3])
            }
        }

        pub struct FetchRequestJob {
            pub url: String,
        }

        impl crate::Job for FetchRequestJob {
            type Outcome = Result<Vec<u8>, Error>;

            fn name(&self) -> String {
                format!("Fetching request: '{}'", URL)
            }

            async fn perform(self, _ctx: crate::Context) -> Self::Outcome {
                fetch(&self.url).await
            }
        }
    }
}
