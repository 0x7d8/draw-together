use std::sync::Arc;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Debug)]
pub enum Action {
    Erase,
    DrawCubeNormal,
    DrawCubeHollow,
    DrawCircleNormal,
    DrawCircleHollow,
    DrawTriangleNormal,
    DrawTriangleHollow,
}

const RESOLUTION: usize = RESOLUTION_WIDTH * RESOLUTION_HEIGHT;
const RESOLUTION_HEIGHT: usize = 1000;
const RESOLUTION_WIDTH: usize = 1920;

// binary format:
// 0-0: action (3b) + height (5b)
// 1-3: x (12b) + y (12b)
// 4-6: color

#[derive(Debug)]
pub struct ClientMessage {
    pub action: Action,

    pub x: u16,
    pub y: u16,

    pub height: u8,
    pub color: [u8; 3],
}

impl ClientMessage {
    fn decode_u12(data: &[u8]) -> [u16; 2] {
        let mut buf: [u16; 2] = [0; 2];

        buf[0] = (data[0] as u16) | ((data[1] as u16 & 0xf) << 8);
        buf[1] = ((data[1] as u16) >> 4) | ((data[2] as u16) << 4);

        buf
    }

    fn encode_u12(data: [u16; 2]) -> [u8; 3] {
        let mut buf = [0; 3];

        buf[0] = (data[0] & 0xff) as u8;
        buf[1] = ((data[0] >> 8) | (data[1] << 4)) as u8;
        buf[2] = (data[1] >> 4) as u8;

        buf
    }

    fn decode_u3(data: u8) -> u8 {
        data & 0b111
    }

    fn encode_u3(data: u8) -> u8 {
        data & 0b111
    }

    fn decode_u5(data: u8) -> u8 {
        data >> 3
    }

    fn encode_u5(data: u8) -> u8 {
        data << 3
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() != 7 {
            return None;
        }

        let action_height = data[0];
        let action = match Self::decode_u3(action_height) {
            0 => Action::Erase,
            1 => Action::DrawCubeNormal,
            2 => Action::DrawCubeHollow,
            3 => Action::DrawCircleNormal,
            4 => Action::DrawCircleHollow,
            5 => Action::DrawTriangleNormal,
            6 => Action::DrawTriangleHollow,
            _ => return None,
        };

        let height = Self::decode_u5(action_height);

        let [x, y] = Self::decode_u12(&data[1..4]);
        let color = [data[4], data[5], data[6]];

        if height == 0 || x >= RESOLUTION_WIDTH as u16 || y >= RESOLUTION_HEIGHT as u16 {
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

    pub fn encode(&self) -> [u8; 7] {
        let mut buf = [0; 7];

        buf[0] = Self::encode_u3(match self.action {
            Action::Erase => 0,
            Action::DrawCubeNormal => 1,
            Action::DrawCubeHollow => 2,
            Action::DrawCircleNormal => 3,
            Action::DrawCircleHollow => 4,
            Action::DrawTriangleNormal => 5,
            Action::DrawTriangleHollow => 6,
        }) | Self::encode_u5(self.height);
        buf[1..4].copy_from_slice(&Self::encode_u12([self.x, self.y]));
        buf[4..7].copy_from_slice(&self.color);

        if std::env::var("DEBUG").is_ok() {
            println!("encoded: {:?}", &self);
        }

        buf
    }

    pub fn encode_internal(&self) -> u32 {
        let mut buf = [0; 4];

        buf[0] = Self::encode_u3(match self.action {
            Action::Erase => 0,
            Action::DrawCubeNormal => 1,
            Action::DrawCubeHollow => 2,
            Action::DrawCircleNormal => 3,
            Action::DrawCircleHollow => 4,
            Action::DrawTriangleNormal => 5,
            Action::DrawTriangleHollow => 6,
        }) | Self::encode_u5(self.height);
        buf[1..4].copy_from_slice(&self.color);

        u32::from_le_bytes(buf)
    }
}

pub struct Data {
    pub data: Arc<Mutex<Vec<u32>>>,
    pub listeners: Vec<tokio::sync::mpsc::Sender<Vec<u8>>>,
}

impl Data {
    pub async fn new(path: Option<&str>, save: bool) -> Self {
        let mut file = match path {
            Some(path) => Some(if save {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)
                    .await
                    .unwrap()
            } else {
                File::open(path).await.unwrap()
            }),
            None => None,
        };

        let mut data: Vec<u32> = vec![0; RESOLUTION];
        if file.is_some() {
            let mut file_data: Vec<u8> = Vec::with_capacity(RESOLUTION * 4);
            file.as_mut()
                .unwrap()
                .read_to_end(&mut file_data)
                .await
                .unwrap();

            data.iter_mut()
                .zip(file_data.chunks_exact(4))
                .for_each(|(data, chunk)| {
                    *data = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                });
        }

        let data = Arc::new(Mutex::new(data));
        let task_data = Arc::clone(&data);
        if file.is_some() && save {
            tokio::spawn(async move {
                #[allow(clippy::unnecessary_unwrap)]
                let mut file = file.unwrap();

                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                    println!("saving data...");

                    file.seek(std::io::SeekFrom::Start(0)).await.unwrap();

                    let data = task_data.lock().await;
                    file.write_all(
                        &data
                            .iter()
                            .flat_map(|data| data.to_le_bytes())
                            .collect::<Vec<u8>>(),
                    )
                    .await
                    .unwrap();

                    println!("saving data... done");

                    file.sync_all().await.unwrap();
                }
            });
        }

        Self {
            data,
            listeners: Vec::new(),
        }
    }

    pub fn add_listener(&mut self, listener: tokio::sync::mpsc::Sender<Vec<u8>>) {
        self.listeners.push(listener);
    }

    pub fn sync_listeners(&mut self) {
        self.listeners.retain(|listener| !listener.is_closed());
    }

    pub async fn write(&mut self, data: &[ClientMessage]) {
        let self_data = Arc::clone(&self.data);
        let mut self_data = self_data.lock().await;

        for message in data {
            let index = (message.y as usize * RESOLUTION_WIDTH) + message.x as usize;
            self_data[index] = message.encode_internal();
        }

        let mut encoded = Vec::with_capacity(7 * data.len());
        for message in data {
            encoded.extend_from_slice(&message.encode());
        }

        for listener in &self.listeners {
            if listener.is_closed() {
                continue;
            }

            listener.send(encoded.to_vec()).await.unwrap();
        }
    }
}
