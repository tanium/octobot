use std::fs;
use std::io;
use std::io::Seek;
use std::net::SocketAddr;
use std::sync::{Arc, mpsc};
use std::thread;

use hyper::server::Http;
use rustls;
use rustls::internal::pemfile;
use tokio_core::reactor::Core;
use tokio_proto;
use tokio_rustls;

use config::Config;
use errors::*;
use github;
use github::api::GithubSession;
use jira;
use jira::api::JiraSession;
use server::github_handler::GithubHandlerState;
use server::octobot_service::OctobotService;
use server::redirect_service::RedirectService;
use server::sessions::Sessions;

pub fn start(config: Config) -> Result<()> {
    let config = Arc::new(config);

    let num_http_threads = config.main.num_http_threads.unwrap_or(20);

    let mut main_threads = vec![];

    let (core_tx, core_rx) = mpsc::channel();
    main_threads.push(thread::Builder::new().name("octobot-core".to_string()).spawn(move || {
        let mut core = Core::new().expect("core");

        core_tx.send(core.remote()).expect("send core handle");

        loop {
            core.turn(None);
        }
    }));
    let core_remote = core_rx.recv().expect("recv core handle");

    let github: Arc<github::api::Session> =
        match GithubSession::new(core_remote.clone(), &config.github.host, &config.github.api_token) {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        };

    let jira: Option<Arc<jira::api::Session>>;
    if let Some(ref jira_config) = config.jira {
        jira = match JiraSession::new(core_remote.clone(), &jira_config) {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => panic!("Error initiating jira session: {}", e),
        };
    } else {
        jira = None;
    }

    let http_addr: SocketAddr = match config.main.listen_addr {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3000".parse().unwrap(),
    };

    let https_addr: SocketAddr = match config.main.listen_addr_ssl {
        Some(ref addr_and_port) => addr_and_port.parse().unwrap(),
        None => "0.0.0.0:3001".parse().unwrap(),
    };

    let tls;
    if let Some(ref cert_file) = config.main.ssl_cert_file {
        if let Some(ref key_file) = config.main.ssl_key_file {
            let key = load_private_key(key_file);
            let certs = load_certs(cert_file);

            let mut the_cfg = rustls::ServerConfig::new();
            the_cfg.set_single_cert(certs, key);

            tls = Some(Arc::new(the_cfg));
        } else {
            warn!("Warning: No SSL configured");
            tls = None;
        }
    } else {
        warn!("Warning: No SSL configured");
        tls = None;
    }

    let ui_sessions = Arc::new(Sessions::new());
    let github_handler_state =
        Arc::new(GithubHandlerState::new(config.clone(), github.clone(), jira.clone(), core_remote.clone()));

    let main_service = move || {
        Ok(OctobotService::new(config.clone(), ui_sessions.clone(), github_handler_state.clone(), core_remote.clone()))
    };
    match tls {
        Some(tls) => {
            main_threads.push(thread::Builder::new().name("https-main".to_string()).spawn(move || {
                let mut server =
                    tokio_proto::TcpServer::new(tokio_rustls::proto::Server::new(Http::new(), tls), https_addr.clone());
                server.threads(num_http_threads);
                info!("Listening (HTTPS) on {}", https_addr);
                server.serve(main_service);
            }));
            main_threads.push(thread::Builder::new().name("http-main".to_string()).spawn(move || {
                let server = Http::new().bind(&http_addr, move || Ok(RedirectService::new(https_addr.port()))).unwrap();
                info!("Listening (HTTP Redirect) on {}", http_addr);
                server.run().unwrap();
            }));
        }
        None => {
            main_threads.push(thread::Builder::new().name("http-main".to_string()).spawn(move || {
                let mut server = tokio_proto::TcpServer::new(Http::new(), http_addr.clone());
                server.threads(num_http_threads);
                info!("Listening (HTTP) on {}", http_addr);
                server.serve(main_service);
            }));
        }
    };

    // run the main threads!
    main_threads
        .into_iter()
        .map(|t| t.unwrap())
        .map(|t| t.join().unwrap())
        .for_each(drop);

    Ok(())
}

fn load_certs(filename: &str) -> Vec<rustls::Certificate> {
    let certfile = fs::File::open(filename).expect("cannot open certificate file");
    let mut reader = io::BufReader::new(certfile);
    pemfile::certs(&mut reader).unwrap()
}

fn load_private_key(filename: &str) -> rustls::PrivateKey {
    let keyfile = fs::File::open(filename).expect("cannot open private key file");
    let mut reader = io::BufReader::new(keyfile);

    let keys = pemfile::rsa_private_keys(&mut reader).unwrap();
    if keys.len() == 1 {
        return keys[0].clone();
    }

    reader.seek(io::SeekFrom::Start(0)).unwrap();
    let keys = pemfile::pkcs8_private_keys(&mut reader).unwrap();
    if keys.len() == 1 {
        return keys[0].clone();
    }

    panic!("Unable to find private key in file {}", filename);
}
