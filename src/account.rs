
use crate::session::BindType;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::sync::atomic::Ordering;
use std::{collections::HashMap, sync::atomic::AtomicBool};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Account {
    pub system_id: String,
    pub password: String,
    pub bind_type: BindType,
    pub multipart_strategy: String,
}

pub trait AccountService: Send + Sync {
    fn find(&self, system_id: &str) -> Result<Option<Account>, AccSrvError>;
}

pub enum AccSrvError {}

impl fmt::Display for AccSrvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AccSrvError display is not implemented yet")
    }
}

pub struct FileBasedAccountService {
    file_path: String,
    is_initialized: AtomicBool,
    accounts: HashMap<String, Account>,
}
unsafe impl Send for FileBasedAccountService {}

impl FileBasedAccountService {
    pub fn new(file_path: String) -> Self {
        let mut service = FileBasedAccountService {
            file_path,
            is_initialized: AtomicBool::new(false),
            accounts: HashMap::new(),
        };
        service.initialize();
        service
    }

    pub fn initialize(&mut self) {
        let accounts_map = self.load_data();
        let is_initialized = self
            .is_initialized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_or_else(|_| false, |b| b);

        if !is_initialized {
            self.accounts = accounts_map;
        }
    }

    fn load_data(&mut self) -> HashMap<String, Account> {
        match fs::read_to_string(self.file_path.as_str()) {
            Err(e) => {
                panic!("Failed to load data from {e:?}");
            }
            Ok(file_content) => {
                let deserialized: Result<Vec<Account>, serde_json::Error> =
                    serde_json::from_str(file_content.as_str());

                match deserialized {
                    Err(e) => panic!("{}", e.to_string()),
                    Ok(_accounts) => {
                        let mut accounts_map: HashMap<String, Account> = HashMap::new();

                        for account in _accounts.iter() {
                            accounts_map.insert(account.system_id.clone(), (*account).clone());
                        }
                        accounts_map
                    }
                }
            }
        }
    }
}

impl AccountService for FileBasedAccountService {
    fn find(&self, system_id: &str) -> Result<Option<Account>, AccSrvError> {
        // AccSrvError will be used when using some DB instead in memory map
        Ok(self.accounts.get(system_id).cloned())
    }
}


