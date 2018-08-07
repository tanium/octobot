use std::fs;
use std::io;
use std::io::Seek;
use std::net::SocketAddr;
use std::sync::Arc;

use futures::{Future, Stream};
use hyper::server::Server;
use rustls;
use rustls::internal::pemfile;
use tokio;
use tokio_rustls::ServerConfigExt;

use config::Config;
use github;
use jira;
use jira::api::JiraSession;
use runtime;
use server::github_handler::GithubHandlerState;
use server::octobot_service::OctobotService;
use server::redirect_service::RedirectService;
use server::sessions::Sessions;

pub fn start(config: Config) {
    let num_http_threads = config.main.num_http_threads.unwrap_or(20);

    runtime::run(num_http_threads, move || run_server(config));
}

fn run_server(config: Config) {
    let config = Arc::new(config);


    let github: Arc<github::api::GithubSessionFactory>;

    if config.github.app_id.is_some() {
        github = match github::api::GithubApp::new(
            &config.github.host,
            config.github.app_id.expect("expected an app_id"),
            &config.github.app_key().expect("expected an app_key"),
        ) {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        };
    } else {
        github = match github::api::GithubOauthApp::new(
            &config.github.host,
            &config.github.api_token.as_ref().expect("expected an api_token"),
        ) {
            Ok(s) => Arc::new(s),
            Err(e) => panic!("Error initiating github session: {}", e),
        };
    }

    let jira: Option<Arc<jira::api::Session>>;
    if let Some(ref jira_config) = config.jira {
        jira = match JiraSession::new(&jira_config) {
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

    let tls_cfg;
    if let Some(ref cert_file) = config.main.ssl_cert_file {
        if let Some(ref key_file) = config.main.ssl_key_file {
            let key = load_private_key(key_file);
            let certs = load_certs(cert_file);

            let mut the_cfg = rustls::ServerConfig::new(rustls::NoClientAuth::new());
            the_cfg.set_single_cert(certs, key);

            tls_cfg = Some(Arc::new(the_cfg));
        } else {
            warn!("Warning: No SSL configured");
            tls_cfg = None;
        }
    } else {
        warn!("Warning: No SSL configured");
        tls_cfg = None;
    }

    let ui_sessions = Arc::new(Sessions::new());
    let github_handler_state = Arc::new(GithubHandlerState::new(config.clone(), github.clone(), jira.clone()));

    let main_service = OctobotService::new(config.clone(), ui_sessions.clone(), github_handler_state.clone());
    let redirect_service = RedirectService::new(https_addr.port());

    if let Some(tls_cfg) = tls_cfg {
        // setup main service on https
        {
            let tcp = tokio::net::TcpListener::bind(&https_addr).unwrap();
            let tls = tcp.incoming()
                .and_then(move |s| tls_cfg.accept_async(s))
                .then(|r| match r {
                    Ok(x) => Ok::<_, io::Error>(Some(x)),
                    Err(_e) => Err(_e),
                })
                .filter_map(|x| x);
            let server = Server::builder(tls).serve(main_service).map_err(|e| error!("server error: {}", e));
            info!("Listening (HTTPS) on {}", https_addr);
            tokio::spawn(server);
        }
        // setup http redirect
        {
            let server = Server::bind(&http_addr).serve(redirect_service).map_err(
                |e| error!("server error: {}", e),
            );
            info!("Listening (HTTP Redirect) on {}", http_addr);
            tokio::spawn(server);
        }
    } else {
        // setup main service on http
        {
            let server = Server::bind(&http_addr).serve(main_service).map(|_| ()).map_err(
                |e| error!("server error: {}", e),
            );
            info!("Listening (HTTP) on {}", http_addr);
            tokio::spawn(server);
        }
    }
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
