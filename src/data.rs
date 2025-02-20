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
}

pub struct Data {
    pub data: Arc<Mutex<Vec<u8>>>,
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

        let mut data: Vec<u8> = vec![0xff; RESOLUTION * 3];
        if file.is_some() {
            file.as_mut().unwrap().read_to_end(&mut data).await.unwrap();
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
            match message.action {
                Action::Erase => {
                    let height = (message.height as f64) * 1.5 * 4.0;

                    let start_x = message.x.saturating_sub(height as u16) as usize;
                    let end_x =
                        ((message.x + height as u16).min(RESOLUTION_WIDTH as u16 - 1)) as usize;
                    let start_y = message.y.saturating_sub(height as u16) as usize;
                    let end_y =
                        ((message.y + height as u16).min(RESOLUTION_HEIGHT as u16 - 1)) as usize;

                    for y in start_y..=end_y {
                        let row_start = y * RESOLUTION_WIDTH * 3;
                        for x in start_x..=end_x {
                            let index = row_start + x * 3;
                            self_data[index] = 0xff;
                            self_data[index + 1] = 0xff;
                            self_data[index + 2] = 0xff;
                        }
                    }
                }
                Action::DrawCubeNormal => {
                    let height = message.height as usize * 4;
                    println!("height: {}", height);
                    let start_x = message.x as usize;
                    let end_x =
                        ((message.x + height as u16).min(RESOLUTION_WIDTH as u16 - 1)) as usize;
                    let start_y = message.y as usize;
                    let end_y =
                        ((message.y + height as u16).min(RESOLUTION_HEIGHT as u16 - 1)) as usize;

                    for y in start_y..=end_y {
                        let row_start = y * RESOLUTION_WIDTH * 3;
                        for x in start_x..=end_x {
                            let index = row_start + x * 3;
                            self_data[index..index + 3].copy_from_slice(&message.color);
                        }
                    }
                }
                Action::DrawCubeHollow => {
                    let height = message.height as usize * 4;
                    let start_x = message.x as usize;
                    let end_x =
                        ((message.x + height as u16).min(RESOLUTION_WIDTH as u16 - 1)) as usize;
                    let start_y = message.y as usize;
                    let end_y =
                        ((message.y + height as u16).min(RESOLUTION_HEIGHT as u16 - 1)) as usize;

                    for x in start_x..=end_x {
                        let top_index = start_y * RESOLUTION_WIDTH * 3 + x * 3;
                        let bottom_index = end_y * RESOLUTION_WIDTH * 3 + x * 3;
                        self_data[top_index..top_index + 3].copy_from_slice(&message.color);
                        self_data[bottom_index..bottom_index + 3].copy_from_slice(&message.color);
                    }

                    for y in start_y..=end_y {
                        let left_index = y * RESOLUTION_WIDTH * 3 + start_x * 3;
                        let right_index = y * RESOLUTION_WIDTH * 3 + end_x * 3;
                        self_data[left_index..left_index + 3].copy_from_slice(&message.color);
                        self_data[right_index..right_index + 3].copy_from_slice(&message.color);
                    }
                }
                Action::DrawCircleNormal | Action::DrawCircleHollow => {
                    let radius = message.height as usize * 4;
                    let is_hollow = matches!(message.action, Action::DrawCircleHollow);

                    let start_x = message.x.saturating_sub(radius as u16) as usize;
                    let end_x =
                        ((message.x + radius as u16).min(RESOLUTION_WIDTH as u16 - 1)) as usize;
                    let start_y = message.y.saturating_sub(radius as u16) as usize;
                    let end_y =
                        ((message.y + radius as u16).min(RESOLUTION_HEIGHT as u16 - 1)) as usize;

                    let center_x = message.x as f32;
                    let center_y = message.y as f32;
                    let radius_sq = (radius * radius) as f32;
                    let inner_radius_sq = ((radius - 1) * (radius - 1)) as f32;

                    for y in start_y..=end_y {
                        let dy = y as f32 - center_y;
                        let dy_sq = dy * dy;
                        let row_start = y * RESOLUTION_WIDTH * 3;

                        for x in start_x..=end_x {
                            let dx = x as f32 - center_x;
                            let dist_sq = dx * dx + dy_sq;

                            if (!is_hollow && dist_sq <= radius_sq)
                                || (is_hollow && dist_sq <= radius_sq && dist_sq >= inner_radius_sq)
                            {
                                let index = row_start + x * 3;
                                self_data[index..index + 3].copy_from_slice(&message.color);
                            }
                        }
                    }
                }
                Action::DrawTriangleNormal | Action::DrawTriangleHollow => {
                    let height = message.height as usize * 4;
                    let is_hollow = matches!(message.action, Action::DrawTriangleHollow);

                    let x1 = message.x as i32;
                    let y1 = message.y as i32;
                    let x2 = (message.x as i32) - (height as i32);
                    let y2 = message.y as i32 + (height as i32 * 2);
                    let x3 = message.x as i32 + height as i32;
                    let y3 = y2;

                    if is_hollow {
                        draw_line_fast(&mut self_data, x1, y1, x2, y2, &message.color);
                        draw_line_fast(&mut self_data, x2, y2, x3, y3, &message.color);
                        draw_line_fast(&mut self_data, x3, y3, x1, y1, &message.color);
                    } else {
                        let min_x = x2.min(x3).min(x1).max(0) as usize;
                        let max_x = x2.max(x3).max(x1).min(RESOLUTION_WIDTH as i32 - 1) as usize;
                        let min_y = y1.min(y2).min(y3).max(0) as usize;
                        let max_y = y1.max(y2).max(y3).min(RESOLUTION_HEIGHT as i32 - 1) as usize;

                        for y in min_y..=max_y {
                            let row_start = y * RESOLUTION_WIDTH * 3;
                            for x in min_x..=max_x {
                                if point_in_triangle_fast(
                                    x as i32, y as i32, x1, y1, x2, y2, x3, y3,
                                ) {
                                    let index = row_start + x * 3;
                                    self_data[index..index + 3].copy_from_slice(&message.color);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !self.listeners.is_empty() {
            let mut encoded = Vec::with_capacity(7 * data.len());
            encoded.extend(data.iter().flat_map(|msg| msg.encode()));

            self.listeners.retain(|listener| !listener.is_closed());

            for listener in &self.listeners {
                listener.send(encoded.clone()).await.unwrap();
            }
        }
    }
}

#[inline(always)]
fn draw_line_fast(data: &mut Vec<u8>, x1: i32, y1: i32, x2: i32, y2: i32, color: &[u8; 3]) {
    draw_single_line(data, x1, y1, x2, y2, color);
    draw_single_line(data, x1 + 1, y1, x2 + 1, y2, color);
    draw_single_line(data, x1, y1 + 1, x2, y2 + 1, color);
    draw_single_line(data, x1 + 1, y1 + 1, x2 + 1, y2 + 1, color);
}

#[inline(always)]
fn draw_single_line(
    data: &mut Vec<u8>,
    mut x1: i32,
    mut y1: i32,
    x2: i32,
    y2: i32,
    color: &[u8; 3],
) {
    let dx = (x2 - x1).abs();
    let dy = -(y2 - y1).abs();
    let sx = if x1 < x2 { 1 } else { -1 };
    let sy = if y1 < y2 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x1 >= 0 && x1 < RESOLUTION_WIDTH as i32 && y1 >= 0 && y1 < RESOLUTION_HEIGHT as i32 {
            let index = (y1 as usize * RESOLUTION_WIDTH + x1 as usize) * 3;
            data[index..index + 3].copy_from_slice(color);
        }

        if x1 == x2 && y1 == y2 {
            break;
        }

        let e2 = err * 2;
        if e2 >= dy {
            err += dy;
            x1 += sx;
        }
        if e2 <= dx {
            err += dx;
            y1 += sy;
        }
    }
}

#[inline(always)]
fn point_in_triangle_fast(
    px: i32,
    py: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    x3: i32,
    y3: i32,
) -> bool {
    let edge1 = (px - x1) * (y2 - y1) - (py - y1) * (x2 - x1);
    let edge2 = (px - x2) * (y3 - y2) - (py - y2) * (x3 - x2);
    let edge3 = (px - x3) * (y1 - y3) - (py - y3) * (x1 - x3);

    (edge1 >= 0 && edge2 >= 0 && edge3 >= 0) || (edge1 <= 0 && edge2 <= 0 && edge3 <= 0)
}
