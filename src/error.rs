#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IO(#[from] std::io::Error),

    #[error(transparent)]
    Ssh(#[from] ssh2::Error),

    #[error("exec command [{0}] error, message: [{1}]")]
    SshCommandError(String, String),

    #[error("thread error")]
    ThreadError(),
}
