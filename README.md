# bevy_jobs

A lightweight job framework for Bevy.

This crate provides a simple way to define, spawn, and manage jobs that run in the background. It is designed to be extensible, allowing you to create your own job types and runners.

It also includes `bevy_job_fetch`, a crate that provides a generic `Job` for fetching data from a URL.

## Getting started

Add the `JobSchedulerPlugin` to your Bevy app:

```rust
use bevy::prelude::*;
use bevy_jobs::JobSchedulerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(JobSchedulerPlugin::default())
        // ... add your systems
        .run();
}
```

### Defining a job

To define a job, you need to implement the `bevy_jobs::Job` trait.

```rust
pub struct MyJob {
    pub some_data: String,
}

impl bevy_jobs::Job for MyJob {
    type Outcome = Result<String, std::io::Error>;

    fn name(&self) -> String {
        format!("MyJob with data: {}", self.some_data)
    }

    async fn perform(self, ctx: bevy_jobs::Context) -> Self::Outcome {
        // Do some work...
        Ok(format!("Job finished with data: {}", self.some_data))
    }
}
```

### Spawning a job

You can spawn a job from any system using the `JobSpawner` resource.

```rust
fn some_spawn_system(
    mut job_spawner: bevy_jobs::JobSpawner,
) {
    job_spawner.spawn(MyJob {
        some_data: "hello".to_string(),
    });
}
```

### Handling job results

When a job is finished, its result can be retrieved from the `FinishedJobs` resource.

```rust
fn some_result_system(
    mut finished_jobs: bevy_jobs::FinishedJobs,
) {
    while let Some(result) = finished_jobs.take_next::<MyJob>() {
        match result {
            Ok(output) => println!("Job succeeded: {}", output),
            Err(e) => eprintln!("Job failed: {}", e),
        }
    }
}
```

### Querying in-progress jobs

You can query for jobs that are currently running.

```rust
fn render_in_progress(
    query: Query<&bevy_jobs::InProgressJob>,
) {
    for job in &query {
        println!("Job '{}' is running", job.name);
    }
}
```

## `bevy_job_fetch`

This is a crate that provides a generic `Job` for fetching data from a URL.

### Usage

The `NetworkFetchJob` is generic over a `UserData` type, which can be any type that implements `Clone + Send + Sync + 'static`. This allows you to pass arbitrary data along with the fetch request.

Here is an example of how to fetch a file and handle the result:

```rust
use bevy::prelude::*;
use bevy_jobs::{JobSpawner, FinishedJobs};
use bevy_jobs::job_fetch::{NetworkFetchJob, FetchedFile};

#[derive(Clone, Send, Sync, Debug, 'static)]
struct MyUserData {
    id: u32,
}

fn setup(mut job_spawner: JobSpawner) {
    let user_data = MyUserData { id: 42 };

    job_spawner.spawn(NetworkFetchJob {
        url: "https://www.rust-lang.org/static/images/rust-logo-blk.svg".to_string(),
        user_data,
        name: "Rust Logo".to_string(),
    });
}

fn handle_completed_jobs(
    mut finished_jobs: FinishedJobs,
) {
    while let Some(result) = finished_jobs.take_next::<NetworkFetchJob<MyUserData>>() {
        match result {
            Ok(fetched_file) => {
                println!(
                    "Fetched file '{}' ({} bytes) with user data: {:?}",
                    fetched_file.name,
                    fetched_file.bytes.len(),
                    fetched_file.user_data
                );
            }
            Err(e) => {
                eprintln!("Error fetching file: {}", e);
            }
        }
    }
}
