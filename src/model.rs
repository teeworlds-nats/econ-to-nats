use std::option::Option;
use std::error::Error;
use std::net::{SocketAddr, ToSocketAddrs};
use std::process::exit;
use serde_derive::{Deserialize, Serialize};
use async_nats::{Client, ConnectOptions, Error as NatsError};
use log::{debug, error};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use regex::Regex;
use nestify::nest;
use crate::util::errors::ConfigError;
use crate::util::utils::get_path;


#[derive(Clone, Serialize, Deserialize)]
pub enum StringOrVecString {
    Single(String),
    Multiple(Vec<String>),
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MsgUtil {
    pub server_name: String,
    pub rcon: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgHandler {
    pub server_name: Option<String>,
    pub name: Option<String>,
    pub message_thread_id: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgBridge {
    pub server_name: String,
    pub message_thread_id: String,
    pub text: String,
}


#[derive(Default)]
pub struct RconData {
    pub text: Option<String>,
    pub sync: bool,
    pub log: bool
}

nest! {
    pub struct EnvHandler {
        pub paths: Vec<
            pub struct HandlerPaths {
                pub read: String,
                pub regex: Vec<Regex>,
                pub write: Vec<String>,
                pub template: String
            }>,
        pub text: String,
        pub text_leave: String,
        pub text_join: String,
        pub text_edit_nickname: String,
        pub nickname_regex: Vec<(Regex, String)>,
        pub block_text_in_nickname: Vec<(String, String)>,
        pub chat_regex: Vec<(Regex, String)>,
        pub block_text_in_chat: Vec<(String, String)>,
    }
}

nest! {
    #[derive(Clone, Deserialize)]
    pub struct Env {
        pub nats:
            #[derive(Clone, Deserialize)]
            pub struct EnvNats {
                pub server: String,
                pub user: Option<String>,
                pub password: Option<String>,

                // Econ
                pub read_path: Option<StringOrVecString>,
                pub write_path: Option<StringOrVecString>,

                // Handler
                pub paths: Option<Vec<
                    #[derive(Clone, Deserialize)]
                    pub struct NatsHandlerPaths {
                        pub read: Option<String>,
                        pub regex: Option<StringOrVecString>,
                        pub write: Option<StringOrVecString>,
                        pub template: Option<String>
                    }>>,
            },

        // econ

        pub check_status_econ: Option<u64>, // In Sec
        pub message_thread_id: Option<String>,
        pub server_name: Option<String>,
        pub econ: Option<
            #[derive(Clone, Deserialize)]
            pub struct EnvEcon {
                pub host: Option<String>,
                pub password: Option<String>,
                pub auth_message: Option<String>,
            }>,

        // handler
        pub text: Option<String>,
        pub text_leave: Option<String>,
        pub text_join: Option<String>,
        pub text_edit_nickname: Option<String>,
        pub nickname_regex: Option<Vec<(String, String)>>,
        pub block_text_in_nickname: Option<Vec<(String, String)>>,
        pub chat_regex: Option<Vec<(String, String)>>,
        pub block_text_in_chat: Option<Vec<(String, String)>>,

        // util-handler
        pub commands: Option<
            #[derive(Clone, Deserialize)]
            pub struct Commands {
                sync: Option<Vec<String>>,
                log: Option<Vec<String>>
            }>,
    }
}

impl From<NatsHandlerPaths> for HandlerPaths {
    fn from(item: NatsHandlerPaths) -> Self {
        let read = item.read.unwrap();
        let regex= get_path(item.regex, vec!())
            .iter()
            .map(|x| Regex::new(x).unwrap())
            .collect();
        let write= get_path(item.write, vec!());
        let template = item.template.unwrap_or_default();

        HandlerPaths { read, regex, write, template }
    }
}

impl HandlerPaths {
    pub fn new(regex: &str) -> Self {
        Self {
            read: "".to_string(),
            regex: vec!(Regex::new(regex).unwrap()),
            write: Vec::new(),
            template: "".to_string()
        }
    }
}


impl Env {
    pub async fn get_yaml() -> Result<Self, ConfigError> {
        let mut contents = String::new();

        File::open("config.yaml").await?.read_to_string(&mut contents).await?;

        let env: Env = serde_yaml::from_str(&contents).map_err(ConfigError::from)?;
        Ok(env)
    }

    pub fn get_env_handler(&self) -> Result<EnvHandler, Box<dyn Error>> {
        let handler_paths: Vec<HandlerPaths> = self.nats.paths
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|path| HandlerPaths::from(path.clone())) // Преобразование каждого элемента
            .collect();

        Ok(EnvHandler {
            paths: handler_paths,
            text: self.text.clone().unwrap_or_else(|| "{{player}} {{text}}".to_string()),
            text_leave: self.text_leave.clone().unwrap_or_else(|| "has left the game".to_string()),
            text_join: self.text_join.clone().unwrap_or_else(|| "has join the game".to_string()),
            text_edit_nickname: self.text_edit_nickname.clone().unwrap_or_else(|| "{{player}} > {{text}}".to_string()),
            nickname_regex: self.nickname_regex.clone()
                .unwrap_or_default()
                .iter()
                .filter_map(|(k, v)| {
                    Regex::new(k).ok().map(|regex| (regex, v.clone())) // Клонируем v для использования в кортежах
                })
                .collect(),
            block_text_in_nickname: self.block_text_in_nickname.clone()
                .unwrap_or_else(|| vec!(("tw/".to_string(), "".to_string()), ("twitch.tv/".to_string(), "".to_string())))
                .into_iter()
                .collect(),
            chat_regex: self.chat_regex.clone()
                .unwrap_or_default()
                .iter()
                .filter_map(|(k, v)| {
                    Regex::new(k).ok().map(|regex| (regex, v.clone())) // Клонируем v для использования в кортежах
                })
                .collect(),
            block_text_in_chat: self.block_text_in_chat.clone()
                .unwrap_or_default()
                .into_iter()
                .collect()
        })

    }

    pub fn get_commands(&self) -> (Vec<String>, Vec<String>) {
        let default_sync_commands = vec![
            "ban_range".to_string(),
            "ban".to_string(),
            "unban_range".to_string(),
            "unban".to_string(),
            "muteip".to_string(),
        ];

        let default_log_commands = vec![
            "ban".to_string(),
            "ban_range".to_string(),
            "unban".to_string(),
            "unban_range".to_string(),
            "kick".to_string(),
            "muteid".to_string(),
            "muteip".to_string(),
        ];

        let sync_commands = self.commands.as_ref()
            .and_then(|commands| commands.sync.as_ref())
            .cloned()
            .unwrap_or(default_sync_commands);

        let log_commands = self.commands.as_ref()
            .and_then(|commands| commands.log.as_ref())
            .cloned()
            .unwrap_or(default_log_commands);

        (sync_commands, log_commands)
    }

    pub fn get_econ_addr(&self) -> SocketAddr {
        let Some(econ) = self.econ.clone() else {exit(-1)};
        if econ.host.is_none() {
            error!("econ.host must be set");
            exit(1);
        }
        econ.host.unwrap().to_socket_addrs().expect("Error create econ address").next().unwrap()
    }

    pub async fn connect_nats(&self) -> Result<Client, NatsError> {
        let connect = match (self.nats.user.clone(), self.nats.password.clone()) {
            (Some(user), Some(password)) => {
                ConnectOptions::new().user_and_password(user, password)
            },
            _ => {
                ConnectOptions::new()
            }
        };
        let nc = connect
            .ping_interval(std::time::Duration::from_secs(15))
            .connect(&self.nats.server)
            .await?;
        debug!("Connected nats: {}", self.nats.server);
        Ok(nc)
    }
}