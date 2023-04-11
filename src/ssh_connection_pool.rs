use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    result,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use ssh2::Session;

use crate::{
    error::{self, Error},
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
        let mut connections = Vec::<SSHConnection>::new();
        for i in 0..MAX_CONNECTIONS {
            match SSHConnection::new() {
                Some(connection) => connections.push(connection),
                None => {
                    println!("init connection {i} none value, continue");
                    continue;
                }
            }
        }

        Arc::new(Mutex::new(connections))
    };
}

struct SSHConnection {
    session: Session,
}

impl SSHConnection {
    pub fn new() -> Option<Self> {
        let tcp = match TcpStream::connect(SSH_HOST) {
            Ok(value) => value,
            Err(e) => {
                println!("tcp connection error: {:?}", e);
                return None;
            }
        };

        let mut session = match Session::new() {
            Ok(value) => value,
            Err(e) => {
                println!("session create error: {:?}", e);
                return None;
            }
        };
        session.set_tcp_stream(tcp);
        session.handshake().unwrap();
        session.set_timeout(TIMEOUT);
        match session.userauth_password(USER_NAME, PASSWORD) {
            Ok(_) => {}
            Err(e) => {
                println!("user auth error: {:?}", e);
                return None;
            }
        };
        Some(Self { session })
    }

    pub fn ssh(&self, command: &String) -> Result<String> {
        let mut channel = self.session.channel_session()?;
        channel.exec(command.as_str())?;
        let mut err = String::new();
        channel.stderr().read_to_string(&mut err)?;
        let res = if err.is_empty() || err.trim().eq("[sudo] password for nvidia:") {
            let mut buf = String::new();
            channel.read_to_string(&mut buf)?;

            Ok(buf)
        } else {
            Err(Error::SshCommandError(command.clone(), err))
        };
        channel.wait_close()?;

        res
    }

    pub fn scp(&self, content: &String, dest_path: &String) -> Result<String> {
        let remote_file =
            self.session
                .scp_send(Path::new(&dest_path), 0o777, content.len() as u64, None);
        match remote_file {
            Ok(mut remote_file) => {
                remote_file.write_all(content.as_bytes()).unwrap();
                remote_file.send_eof().unwrap();
                remote_file.wait_eof().unwrap();
                remote_file.close().unwrap();
                remote_file.wait_close().unwrap();
                Ok(String::new())
            }
            Err(e) => Err(error::Error::Ssh(e)),
        }
    }

    pub fn ssh_stream(&self, command: String, sender: Sender<String>) -> Result<String> {
        let mut channel = self.session.channel_session()?;
        let t = thread::spawn(move || {
            let mut buf = [0_u8; 1024];
            channel.exec(command.as_str()).unwrap();
            loop {
                match channel.read(&mut buf) {
                    Ok(size) if size > 0 => {
                        sender
                            .send(String::from_utf8_lossy(&buf).to_string())
                            .unwrap();
                        buf.fill(0_u8);
                    }
                    Ok(_) => {
                        println!("end of file");
                        return Ok(String::from("eof"));
                    }
                    Err(e) => {
                        return Err(Error::IO(e));
                    }
                }
            }
        });

        match t.join() {
            Ok(res) => res,
            Err(_) => Err(Error::ThreadError()),
        }
    }

    pub fn authenticated(&self) -> bool {
        self.session.authenticated()
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

    pub fn ssh<C>(&self, command: String, callback: C)
    where
        C: Fn(Result<String>) + Send + 'static,
    {
        let run = move || loop {
            let connection = Self::get_connection();
            match connection.ssh(&command) {
                Ok(result) => {
                    Self::return_connection(connection);
                    break Ok(result);
                }
                Err(e) => {
                    println!("scp file error: {:?}", e);
                }
            }
        };
        let _ = self.thread_pool().execute(run, callback);
    }

    pub fn try_ssh<C>(&self, command: String, callback: C) -> bool
    where
        C: Fn(Result<String>) + Send + 'static,
    {
        let connection = Self::try_get_connection();
        if connection.as_ref().is_none() {
            return false;
        }

        let connection = connection.unwrap();
        let run = move || {
            let result = connection.ssh(&command);
            Self::return_connection(connection);
            result
        };
        let _ = self.thread_pool().execute(run, callback);

        true
    }

    pub fn scp<C>(&self, content: String, dest_path: String, callback: C)
    where
        C: Fn(Result<String>) + Send + 'static,
    {
        let run = move || {
            let result = loop {
                let connection = Self::get_connection();
                thread::sleep(Duration::from_secs(1));
                match connection.scp(&content, &dest_path) {
                    Ok(result) => {
                        Self::return_connection(connection);
                        break Ok(result);
                    }
                    Err(e) => {
                        println!("scp file error: {:?}", e);
                    }
                }
            };
            result
        };
        self.thread_pool().execute(run, callback);
    }

    pub fn try_scp<C>(&self, content: String, dest_path: String, callback: C) -> bool
    where
        C: Fn(Result<String>) + Send + 'static,
    {
        let connection = Self::try_get_connection();
        if connection.is_none() {
            return false;
        }
        let connection = connection.unwrap();
        let run = move || {
            let result = connection.scp(&content, &dest_path);

            Self::return_connection(connection);
            result
        };
        self.thread_pool().execute(run, callback);

        true
    }

    // pub fn ssh_stream<R>(&self, command: String, recv_callback: R)
    // where
    //     R: Fn(Mutex<Receiver<String>>) + Send + 'static,
    // {
    //     let run = move || loop {
    //         let connection = Self::get_connection();
    //         let (sender, receiver) = mpsc::channel::<String>();
    //         let receiver = Mutex::new(receiver);
    //         thread::spawn(|| recv_callback(receiver));
    //         match connection.ssh_stream(command.clone(), sender) {
    //             Ok(res) => {
    //                 Self::return_connection(connection);
    //                 break Ok(res);
    //             }
    //             Err(e) => {
    //                 println!("ssh stream error: {:?}", e);
    //             }
    //         }
    //     };
    //     self.thread_pool.execute(run, |_| {});
    // }

    fn get_connection() -> SSHConnection {
        let mut connections = CONNECTIONS.lock().unwrap();
        if let Some(connection) = connections.pop() {
            if connection.authenticated() {
                connection
            } else {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    if let Some(connection) = SSHConnection::new() {
                        break connection;
                    }
                }
            }
        } else {
            loop {
                thread::sleep(Duration::from_secs(1));
                if let Some(connection) = SSHConnection::new() {
                    if connection.authenticated() {
                        break connection;
                    }
                }
            }
        }
    }

    fn try_get_connection() -> Option<SSHConnection> {
        let mut connections = CONNECTIONS.lock().unwrap();
        if let Some(connection) = connections.pop() {
            if connection.authenticated() {
                return Some(connection);
            }
        }

        None
    }

    fn return_connection(connection: SSHConnection) -> bool {
        if !connection.authenticated() {
            return false;
        }

        let mut connections = CONNECTIONS.lock().unwrap();
        connections.push(connection);

        true
    }
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use std::{
        thread,
        time::{Duration, SystemTime},
    };

    use super::SSHConnectionPool;
    type Result<T> = std::result::Result<T, Error>;

    #[test]
    fn test_ssh() {
        let ssh_connection_pool = SSHConnectionPool::new();

        let callback = |res: Result<String>| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros();
            println!("{}", now);
            println!("exec command result: {:#?}", res);
        };

        ssh_connection_pool.ssh("cat /home/dev/test.txt".to_string(), callback);

        for i in 0..1000 {
            println!("main sleep {i}");
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn test_scp() {
        let ssh_connection_pool = SSHConnectionPool::new();

        let callback = |res: Result<String>| {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros();
            println!("{}", now);
            println!("exec command result: {:#?}", res);
            let ssh_connection_pool = SSHConnectionPool::new();
            ssh_connection_pool.ssh(
                "cat /home/dev/test.txt".to_string(),
                |res: Result<String>| println!("{:?}", res),
            );
        };

        ssh_connection_pool.scp(
            "hello scpssssss".to_string(),
            "/home/dev/test.txt".to_string(),
            callback,
        );

        for i in 0..1000 {
            println!("main sleep {i}");
            thread::sleep(Duration::from_secs(1));
        }
    }
}
