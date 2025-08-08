use bevy_jobs::Job;
use futures_util::StreamExt;
use std::io;

pub struct FetchedFile<T: Clone + Send + Sync + 'static> {
    pub name: String,
    pub bytes: bytes::Bytes,
    pub user_data: T,
}

pub struct NetworkFetchJob<T: Clone + Send + Sync + 'static> {
    pub url: String,
    pub user_data: T,
    pub name: String,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    IoRef(&'static io::Error),
    Reqwest(reqwest::Error),
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO error: {}", e),
            Error::IoRef(e) => write!(f, "IO error: {}", e),
            Error::Reqwest(e) => write!(f, "Reqwest error: {}", e),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<&'static io::Error> for Error {
    fn from(e: &'static io::Error) -> Self {
        Error::IoRef(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Reqwest(e)
    }
}

impl<T: Clone + Send + Sync + 'static> Job for NetworkFetchJob<T> {
    type Outcome = Result<FetchedFile<T>, Error>;
    const JOB_TYPE: bevy_jobs::JobType = bevy_jobs::JobType::Tokio;

    fn name(&self) -> String {
        format!("Fetching '{}'", self.name)
    }

    async fn perform(self, ctx: bevy_jobs::Context) -> Self::Outcome {
        build_request_future(self.url, self.user_data, self.name, ctx).await
    }
}

async fn build_request_future<T: Clone + Send + Sync + 'static>(
    url: String,
    user_data: T,
    name: String,
    ctx: bevy_jobs::Context,
) -> Result<FetchedFile<T>, Error> {
    let response = reqwest::get(url).await?;
    let total_size = response.content_length().unwrap_or(0);
    let mut bytes_stream = response.bytes_stream();
    let mut bytes = Vec::<u8>::with_capacity(total_size as usize);
    let mut last_percent: u8 = 0;

    while let Some(bytes_chunk) = bytes_stream.next().await {
        let mut bytes_chunk = Vec::from(bytes_chunk?);
        bytes.append(&mut bytes_chunk);
        if total_size > 0 {
            let new_percent = (100 * bytes.len() / total_size as usize) as u8;
            if new_percent != last_percent {
                let _ = ctx.send_progress(new_percent).await;
                last_percent = new_percent;
            }
        }
    }

    Ok(FetchedFile {
        bytes: bytes::Bytes::from(bytes),
        user_data,
        name,
    })
}
