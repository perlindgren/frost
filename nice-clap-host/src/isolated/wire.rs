use super::*;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
pub const PROTOCOL: u32 = 1;
#[derive(Serialize, Deserialize, Clone)]
pub struct Class {
    pub descriptor: Descriptor,
    pub inputs: u8,
    pub outputs: u8,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub path: PathBuf,
    pub class: Option<String>,
    pub sample_rate: f64,
    pub max_frames: usize,
    pub inputs: usize,
    pub outputs: usize,
    pub state: Option<Vec<u8>>,
    pub memory: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
pub enum Command {
    Hello {
        version: u32,
        token: String,
        config: Config,
    },
    Parameter {
        id: u32,
        value: f64,
    },
    Save(u64),
    Open {
        position: Option<[i32; 2]>,
        size: [u32; 2],
    },
    Close,
    Shutdown,
}
#[derive(Serialize, Deserialize)]
pub enum Reply {
    Ready {
        classes: Vec<Class>,
        latency: usize,
        parameters: Vec<Parameter>,
        state: Option<Vec<u8>>,
        editor: bool,
    },
    Saved {
        request: u64,
        state: Option<Vec<u8>>,
    },
    State(Option<Vec<u8>>),
    Parameters(Vec<Parameter>),
    Geometry {
        position: Option<[i32; 2]>,
        size: [u32; 2],
    },
    Opened,
    Closed,
    Error(String),
    EditorError(String),
}
pub fn send<T: Serialize>(stream: &mut impl Write, message: &T) -> Result<(), String> {
    let bytes = ron::ser::to_string(message)
        .map_err(|e| e.to_string())?
        .into_bytes();
    if bytes.len() > MAX_MESSAGE {
        return Err("Isolation control message exceeds 16 MiB".into());
    }
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())
}
pub fn receive<T: for<'de> Deserialize<'de>>(stream: &mut impl Read) -> Result<T, String> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_MESSAGE {
        return Err("Invalid isolation control message length".into());
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    ron::from_str(text).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_reject_unbounded_lengths() {
        let mut bytes = Vec::new();
        send(&mut bytes, &Command::Parameter { id: 7, value: 0.25 }).unwrap();
        assert!(matches!(
            receive::<Command>(&mut bytes.as_slice()).unwrap(),
            Command::Parameter { id: 7, value: 0.25 }
        ));
        assert!(receive::<Command>(&mut u32::MAX.to_le_bytes().as_slice()).is_err());
        bytes.pop();
        assert!(receive::<Command>(&mut bytes.as_slice()).is_err());
    }
}
