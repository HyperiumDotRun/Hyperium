use std::path::{Path, PathBuf};

use rodio::{Decoder, MixerDeviceSink, Player};

#[derive(Clone)]
pub struct Track {
    pub title: String,
    path: PathBuf,
}

pub struct AudioPlayer {
    _stream: MixerDeviceSink,
    player: Player,

    playlist: Vec<Track>,
    current: Option<usize>,
    volume: f32,
    paused: bool,
}

impl AudioPlayer {
    pub fn init(music_dir: &Path) -> Option<Self> {
        let stream = rodio::DeviceSinkBuilder::open_default_sink().ok()?;
        let player = Player::connect_new(stream.mixer());
        let volume = 0.6;
        player.set_volume(volume);
        Some(Self {
            _stream: stream,
            player,
            playlist: scan_tracks(music_dir),
            current: None,
            volume,
            paused: false,
        })
    }

    pub fn playlist(&self) -> &[Track] {
        &self.playlist
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_playing(&self) -> bool {
        self.current.is_some() && !self.paused
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|i| self.playlist.get(i))
    }

    pub fn toggle_pause(&mut self) {
        if self.current.is_none() {
            if !self.playlist.is_empty() {
                self.paused = false;
                self.play_index(0);
            }
            return;
        }
        self.paused = !self.paused;
        if self.paused {
            self.player.pause();
        } else {
            self.player.play();
        }
    }

    pub fn next(&mut self) {
        if self.playlist.is_empty() {
            return;
        }
        let i = self.current.map_or(0, |c| (c + 1) % self.playlist.len());
        self.play_index(i);
    }

    pub fn prev(&mut self) {
        if self.playlist.is_empty() {
            return;
        }
        let n = self.playlist.len();
        let i = self.current.map_or(0, |c| (c + n - 1) % n);
        self.play_index(i);
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        self.player.set_volume(self.volume);
    }

    pub fn tick(&mut self) {
        if !self.paused && self.current.is_some() && self.player.empty() {
            self.next();
        }
    }

    fn play_index(&mut self, i: usize) {
        let Some(path) = self.playlist.get(i).map(|t| t.path.clone()) else {
            return;
        };
        let Ok(file) = std::fs::File::open(&path) else {
            return;
        };
        let Ok(source) = Decoder::try_from(file) else {
            return;
        };

        let player = Player::connect_new(self._stream.mixer());
        player.set_volume(self.volume);
        player.append(source);
        if self.paused {
            player.pause();
        }
        self.player = player;
        self.current = Some(i);
    }
}

fn scan_tracks(dir: &Path) -> Vec<Track> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut tracks: Vec<Track> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mp3")))
        .filter_map(|path| {
            let title = path.file_stem()?.to_string_lossy().into_owned();
            Some(Track { title, path })
        })
        .collect();
    tracks.sort_by_key(|t| t.title.to_lowercase());
    tracks
}
