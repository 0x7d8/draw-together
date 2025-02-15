use std::sync::Arc;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

pub enum Action {
    Draw,
    Erase,
}

pub struct ClientMessage {
    pub action: Action,

    pub x: u16,
    pub y: u16,

    pub height: u8,
    pub color: [u8; 3],
}

impl ClientMessage {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() != 9 {
            return None;
        }

        let action = match data[0] {
            0 => Action::Draw,
            1 => Action::Erase,
            _ => return None,
        };

        let x = u16::from_le_bytes([data[1], data[2]]);
        let y = u16::from_le_bytes([data[3], data[4]]);
        let height = data[5];
        let color = [data[6], data[7], data[8]];

        if height == 0 {
            return None;
        }

        Some(Self {
            action,
            x,
            y,
            height,
            color,
        })
    }

    pub fn encode(&self) -> [u8; 9] {
        let mut buf = [0; 9];

        buf[0] = match self.action {
            Action::Draw => 0,
            Action::Erase => 1,
        };

        buf[1..3].copy_from_slice(&self.x.to_le_bytes());
        buf[3..5].copy_from_slice(&self.y.to_le_bytes());
        buf[5] = self.height;
        buf[6..9].copy_from_slice(&self.color);

        buf
    }
}

pub struct Data {
    pub file: Option<File>,
    pub data: Option<Arc<Mutex<Vec<u8>>>>,

    pub listeners: Vec<tokio::sync::mpsc::Sender<Vec<u8>>>,
    save: bool,
}

impl Data {
    pub async fn new(path: Option<&str>, save: bool) -> Self {
        let file = match path {
            Some(path) => Some(if save {
                OpenOptions::new()
                    .read(true)
                    .append(true)
                    .create(true)
                    .open(path)
                    .await
                    .unwrap()
            } else {
                File::open(path).await.unwrap()
            }),
            None => {
                return Self {
                    file: None,
                    data: Some(Arc::new(Mutex::new(Vec::new()))),
                    listeners: Vec::new(),
                    save,
                }
            }
        };

        if !save {
            let mut data = Vec::new();
            file.unwrap().read_to_end(&mut data).await.unwrap();

            return Self {
                file: None,
                data: Some(Arc::new(Mutex::new(data))),
                listeners: Vec::new(),
                save,
            };
        }

        Self {
            file,
            data: None,
            listeners: Vec::new(),
            save,
        }
    }

    pub fn add_listener(&mut self, listener: tokio::sync::mpsc::Sender<Vec<u8>>) {
        self.listeners.push(listener);
    }

    pub fn sync_listeners(&mut self) {
        self.listeners.retain(|listener| !listener.is_closed());
    }

    pub async fn write(&mut self, data: &ClientMessage) {
        let encoded = data.encode();

        if let Some(file) = &mut self.file {
            if !self.save {
                return;
            }

            file.write_all(&encoded).await.unwrap();
        } else {
            let self_data = self.data.as_mut().unwrap();

            let mut self_data = self_data.lock().await;
            self_data.extend_from_slice(&encoded);
        }

        for listener in &self.listeners {
            if listener.is_closed() {
                continue;
            }

            listener.send(encoded.to_vec()).await.unwrap();
        }
    }
}
