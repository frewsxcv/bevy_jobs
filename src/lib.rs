#![warn(
    clippy::unwrap_used,
    clippy::cast_lossless,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::expect_used
)]

use std::{any, future, pin};

pub struct Plugin;

impl bevy_app::Plugin for Plugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_systems(bevy_app::Update, check_system)
            .insert_resource(JobOutcomePayloads(vec![]));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub type AsyncReturn<Output> = pin::Pin<Box<dyn future::Future<Output = Output> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type AsyncReturn<Output> = pin::Pin<Box<dyn future::Future<Output = Output> + 'static>>;

pub trait Job: any::Any + Sized + Send + Sync + 'static {
    type Outcome: any::Any + Send + Sync;

    fn name(&self) -> String;

    fn perform(self, context: Context) -> AsyncReturn<Self::Outcome>;

    fn spawn(self, commands: &mut bevy_ecs::system::Commands) {
        let (outcome_tx, outcome_recv) = async_channel::unbounded::<JobOutcomePayload>();
        let (progress_tx, progress_recv) = async_channel::unbounded::<Progress>();

        let job_name = self.name();
        let in_progress_job = InProgressJob {
            name: job_name.clone(),
            progress: 0,
            progress_recv,
            outcome_recv,
        };

        bevy_tasks::AsyncComputeTaskPool::get()
            .spawn(async move {
                let instant = instant::Instant::now();
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
            })
            .detach();

        commands.spawn(in_progress_job);
    }
}

fn check_system(
    mut query: bevy_ecs::system::Query<(&mut InProgressJob, bevy_ecs::entity::Entity)>,
    mut commands: bevy_ecs::system::Commands,
    mut finished_jobs: FinishedJobs,
) {
    query.for_each_mut(|(mut in_progress_job, entity)| {
        // TODO: Maybe don't run the `try_recv` below every frame?
        if let Ok(progress) = in_progress_job.progress_recv.try_recv() {
            in_progress_job.progress = progress;
        }

        if let Ok(outcome) = in_progress_job.outcome_recv.try_recv() {
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

impl<'w, 's> JobSpawner<'w, 's> {
    pub fn spawn<J: Job>(&mut self, job: J) {
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
    #[system_param(ignore)]
    phantom_data: std::marker::PhantomData<&'s ()>,
}

#[derive(bevy_ecs::system::Resource)]
pub struct JobOutcomePayloads(Vec<JobOutcomePayload>);

impl<'w, 's> FinishedJobs<'w, 's> {
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
