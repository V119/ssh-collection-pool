use std::{
    io::Read,
    net::TcpStream,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use ssh2::Session;

use crate::{
    error::Error,
    thread_pool::{self, ThreadPool},
};

static MAX_CONNECTIONS: usize = 10;
static TIMEOUT: u32 = 5_000;
static SSH_HOST: &str = "10.32.11.177:22";
static USER_NAME: &str = "dev";
static PASSWORD: &str = "LinUx%3412";

type Result<T> = std::result::Result<T, Error>;

lazy_static! {
    static ref CONNECTIONS: Arc<Mutex<Vec<SSHConnection>>> = {
        let mut connections = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            connections.push(SSHConnection::new());
        }

        println!("connection init");

        Arc::new(Mutex::new(connections))
    };
}

struct SSHConnection {
    session: Session,
}

impl SSHConnection {
    pub fn new() -> Self {
        let tcp = TcpStream::connect(SSH_HOST).unwrap();
        let mut session = Session::new().unwrap();
        session.set_tcp_stream(tcp);
        session.handshake().unwrap();
        session.set_timeout(TIMEOUT);
        session.userauth_password(USER_NAME, PASSWORD).unwrap();
        Self { session }
    }

    pub fn ssh(&self, command: String) -> Result<String> {
        let mut channel = self.session.channel_session()?;
        channel.exec(command.as_str())?;
        let mut err = String::new();
        channel.stderr().read_to_string(&mut err)?;
        let res = if err.is_empty() || err.trim().eq("[sudo] password for nvidia:") {
            let mut buf = String::new();
            channel.read_to_string(&mut buf)?;

            Ok(buf)
        } else {
            Err(Error::SshCommandError(command, err))
        };
        channel.wait_close()?;

        res
    }
}

pub struct SSHConnectionPool {
    thread_pool: ThreadPool<Result<String>>,
}

impl SSHConnectionPool {
    pub fn new() -> Self {
        Self {
            thread_pool: thread_pool::ThreadPool::new(10),
        }
    }

    fn thread_pool(&self) -> &ThreadPool<Result<String>> {
        &self.thread_pool
    }

    pub fn ssh(&self, command: String) {
        let run = move || {
            let connection = Self::get_connection();
            let result = connection.ssh(command);
            Self::return_connection(connection);
            result
        };
        let callback = |res: Result<String>| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros();
            println!("{}", now);
            println!("exec command result: {:#?}", res)
        };

        let _ = self.thread_pool().execute(run, callback);
    }

    fn get_connection() -> SSHConnection {
        let mut connections = CONNECTIONS.lock().unwrap();
        if let Some(connection) = connections.pop() {
            println!("connection1");
            connection
        } else {
            println!("connection2");
            SSHConnection::new()
        }
    }

    fn return_connection(connection: SSHConnection) {
        let mut connections = CONNECTIONS.lock().unwrap();
        connections.push(connection);
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::SSHConnectionPool;

    #[test]
    fn test_ssh() {
        let ssh_connection_pool = SSHConnectionPool::new();
        for i in 0..100 {
            ssh_connection_pool.ssh("ls -al".to_string());
        }

        thread::sleep(Duration::from_secs(100));
    }
}
