use std::{path::Path, sync::Arc};
use tokio::{
    fs::File,
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
    DrawHexagonNormal,
    DrawHexagonHollow,
}

const RESOLUTION: usize = RESOLUTION_WIDTH * RESOLUTION_HEIGHT;
const RESOLUTION_HEIGHT: usize = 1000;
const RESOLUTION_WIDTH: usize = 1920;

// binary format:
// (4b) action  | byte 1
// (4b) height  | byte 1
//
// (3b) height  | byte 2
// (5b) x       | byte 2
//
// (6b) x       | byte 3
// (2b) y       | byte 3
//
// (8b) y       | byte 4
//
// (8b) color[0]| byte 5
// (8b) color[1]| byte 6
// (8b) color[2]| byte 7

#[derive(Debug)]
pub struct ClientMessage {
    pub action: Action,

    pub x: u16,
    pub y: u16,

    pub height: u8,
    pub color: [u8; 3],
}

impl ClientMessage {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() != 7 {
            return None;
        }

        let action = match (data[0] >> 4) & 0xF {
            0 => Action::Erase,
            1 => Action::DrawCubeNormal,
            2 => Action::DrawCubeHollow,
            3 => Action::DrawCircleNormal,
            4 => Action::DrawCircleHollow,
            5 => Action::DrawTriangleNormal,
            6 => Action::DrawTriangleHollow,
            7 => Action::DrawHexagonNormal,
            8 => Action::DrawHexagonHollow,
            _ => return None,
        };

        let height_high = data[0] & 0xF;
        let height_low = (data[1] >> 5) & 0x7;
        let height = (height_high << 3) | height_low;
        let x_high = data[1] & 0x1F;
        let x_low = (data[2] >> 2) & 0x3F;
        let x = ((x_high as u16) << 6) | (x_low as u16);
        let y_high = data[2] & 0x3;
        let y = ((y_high as u16) << 8) | (data[3] as u16);
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

        let action_value = match self.action {
            Action::Erase => 0,
            Action::DrawCubeNormal => 1,
            Action::DrawCubeHollow => 2,
            Action::DrawCircleNormal => 3,
            Action::DrawCircleHollow => 4,
            Action::DrawTriangleNormal => 5,
            Action::DrawTriangleHollow => 6,
            Action::DrawHexagonNormal => 7,
            Action::DrawHexagonHollow => 8,
        };

        buf[0] = (action_value << 4) | ((self.height >> 3) & 0xF);
        buf[1] = ((self.height & 0x7) << 5) | ((self.x >> 6) as u8 & 0x1F);
        buf[2] = (((self.x & 0x3F) << 2) | ((self.y >> 8) & 0x3)) as u8;
        buf[3] = self.y as u8;
        buf[4..].copy_from_slice(&self.color);

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
    pub async fn new(path: Option<String>, save: bool) -> Self {
        let mut file = match path.clone() {
            Some(path) => match Path::new(&path).exists() {
                true => Some(File::open(path).await.unwrap()),
                false => None,
            },
            None => None,
        };

        let mut data: Vec<u8> = vec![0xff; RESOLUTION * 3];
        if file.is_some() {
            data.clear();
            file.as_mut().unwrap().read_to_end(&mut data).await.unwrap();
        }

        drop(file);

        let data = Arc::new(Mutex::new(data));
        let task_data = Arc::clone(&data);
        if let Some(path) = path {
            if save {
                tokio::spawn(async move {
                    let mut file = File::options()
                        .write(true)
                        .truncate(true)
                        .create(true)
                        .open(path)
                        .await
                        .unwrap();

                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

                        println!("saving data...");

                        file.seek(tokio::io::SeekFrom::Start(0)).await.unwrap();

                        let data = task_data.lock().await;
                        file.write_all(&data).await.unwrap();

                        drop(data);
                        file.sync_all().await.unwrap();

                        println!("saving data... done");
                    }
                });
            }
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
                    let height = (message.height as f64) * 1.5;

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
                            self_data[index..index + 3].copy_from_slice(&[0, 0, 0]);
                        }
                    }
                }
                Action::DrawCubeNormal => {
                    let height = message.height as usize;

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
                    let height = message.height as usize;

                    let start_x = message.x as usize;
                    let end_x =
                        ((message.x + height as u16).min(RESOLUTION_WIDTH as u16 - 1)) as usize;
                    let start_y = message.y as usize;
                    let end_y =
                        ((message.y + height as u16).min(RESOLUTION_HEIGHT as u16 - 1)) as usize;

                    for offset in 0..2 {
                        for x in start_x..=end_x {
                            if (start_y + offset) < RESOLUTION_HEIGHT {
                                let top_index = (start_y + offset) * RESOLUTION_WIDTH * 3 + x * 3;
                                self_data[top_index..top_index + 3].copy_from_slice(&message.color);
                            }

                            if (end_y + offset) < RESOLUTION_HEIGHT {
                                let bottom_index = (end_y + offset) * RESOLUTION_WIDTH * 3 + x * 3;
                                self_data[bottom_index..bottom_index + 3]
                                    .copy_from_slice(&message.color);
                            }
                        }
                    }

                    for offset in 0..2 {
                        for y in start_y..=end_y {
                            if (start_x + offset) < RESOLUTION_WIDTH {
                                let left_index = y * RESOLUTION_WIDTH * 3 + (start_x + offset) * 3;
                                self_data[left_index..left_index + 3]
                                    .copy_from_slice(&message.color);
                            }

                            if (end_x + offset) < RESOLUTION_WIDTH {
                                let right_index = y * RESOLUTION_WIDTH * 3 + (end_x + offset) * 3;
                                self_data[right_index..right_index + 3]
                                    .copy_from_slice(&message.color);
                            }
                        }
                    }
                }
                Action::DrawCircleNormal | Action::DrawCircleHollow => {
                    let radius = message.height as usize;
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

                    if is_hollow {
                        let outer_radius_sq = radius_sq;
                        let inner_radius_sq = ((radius - 2) * (radius - 2)) as f32;

                        for y in start_y..=end_y {
                            let dy = y as f32 - center_y;
                            let dy_sq = dy * dy;
                            let row_start = y * RESOLUTION_WIDTH * 3;

                            for x in start_x..=end_x {
                                let dx = x as f32 - center_x;
                                let dist_sq = dx * dx + dy_sq;

                                if dist_sq <= outer_radius_sq && dist_sq >= inner_radius_sq {
                                    let index = row_start + x * 3;
                                    self_data[index..index + 3].copy_from_slice(&message.color);
                                }
                            }
                        }
                    } else {
                        for y in start_y..=end_y {
                            let dy = y as f32 - center_y;
                            let dy_sq = dy * dy;
                            let row_start = y * RESOLUTION_WIDTH * 3;

                            for x in start_x..=end_x {
                                let dx = x as f32 - center_x;
                                let dist_sq = dx * dx + dy_sq;

                                if dist_sq <= radius_sq {
                                    let index = row_start + x * 3;
                                    self_data[index..index + 3].copy_from_slice(&message.color);
                                }
                            }
                        }
                    }
                }
                Action::DrawTriangleNormal | Action::DrawTriangleHollow => {
                    let height = message.height as usize;
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
                Action::DrawHexagonNormal | Action::DrawHexagonHollow => {
                    let is_hollow = matches!(message.action, Action::DrawHexagonHollow);
                    let size = message.height as f32;
                    let center_x = message.x as f32;
                    let center_y = message.y as f32;

                    let points = [
                        (center_x + size, center_y),
                        (center_x + size / 2.0, center_y - size),
                        (center_x - size / 2.0, center_y - size),
                        (center_x - size, center_y),
                        (center_x - size / 2.0, center_y + size),
                        (center_x + size / 2.0, center_y + size),
                    ];

                    if is_hollow {
                        for i in 0..6 {
                            let start = points[i];
                            let end = points[(i + 1) % 6];
                            draw_line_fast(
                                &mut self_data,
                                start.0 as i32,
                                start.1 as i32,
                                end.0 as i32,
                                end.1 as i32,
                                &message.color,
                            );
                        }
                    } else {
                        let min_y = points.iter().map(|(_, y)| *y as i32).min().unwrap();
                        let max_y = points.iter().map(|(_, y)| *y as i32).max().unwrap();

                        for y in min_y..=max_y {
                            let mut intersections = Vec::new();

                            for i in 0..6 {
                                let start = points[i];
                                let end = points[(i + 1) % 6];

                                if (start.1 <= y as f32 && end.1 > y as f32)
                                    || (end.1 <= y as f32 && start.1 > y as f32)
                                {
                                    let x = if start.1 == end.1 {
                                        start.0
                                    } else {
                                        start.0
                                            + (y as f32 - start.1) * (end.0 - start.0)
                                                / (end.1 - start.1)
                                    };
                                    intersections.push(x as i32);
                                }
                            }

                            intersections.sort_unstable();

                            for chunk in intersections.chunks(2) {
                                if chunk.len() == 2 {
                                    let start_x = chunk[0].max(0).min(RESOLUTION_WIDTH as i32 - 1);
                                    let end_x = chunk[1].max(0).min(RESOLUTION_WIDTH as i32 - 1);

                                    for x in start_x..=end_x {
                                        let index =
                                            (y as usize * RESOLUTION_WIDTH + x as usize) * 3;
                                        self_data[index..index + 3].copy_from_slice(&message.color);
                                    }
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

            for listener in &self.listeners {
                if listener.is_closed() {
                    continue;
                }

                listener.send(encoded.clone()).await.unwrap();
            }
        }
    }
}

#[inline(always)]
fn draw_line_fast(data: &mut [u8], x1: i32, y1: i32, x2: i32, y2: i32, color: &[u8; 3]) {
    draw_single_line(data, x1, y1, x2, y2, color);
    draw_single_line(data, x1 + 1, y1, x2 + 1, y2, color);
    draw_single_line(data, x1, y1 + 1, x2, y2 + 1, color);
    draw_single_line(data, x1 + 1, y1 + 1, x2 + 1, y2 + 1, color);
}

#[inline(always)]
fn draw_single_line(data: &mut [u8], mut x1: i32, mut y1: i32, x2: i32, y2: i32, color: &[u8; 3]) {
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

#[allow(clippy::too_many_arguments)]
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
