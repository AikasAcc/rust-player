use crate::{
    Player,
    library::Media,
    taskbar::{Buttons, Command, Taskbar},
};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::mpsc::{self, Receiver},
};
use windows::{
    Foundation::{EventRegistrationToken, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    },
    Storage::Streams::{IRandomAccessStream, RandomAccessStreamReference},
    Win32::{
        Foundation::{E_OUTOFMEMORY, HWND},
        System::WinRT::{
            BSOS_DEFAULT, CreateRandomAccessStreamOverStream, ISystemMediaTransportControlsInterop,
            RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize,
        },
        UI::Shell::SHCreateMemStream,
    },
    core::{HSTRING, factory},
};

pub const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;

pub struct MediaKeys {
    controls: SystemMediaTransportControls,
    token: EventRegistrationToken,
    commands: Receiver<Command>,
    state: Option<(bool, bool, Buttons)>,
    metadata_dirty: bool,
    has_metadata: bool,
    _runtime: WinRt,
}

impl MediaKeys {
    pub fn new(window: &impl HasWindowHandle, ctx: &egui::Context) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("No Windows window available".into());
        };
        Self::for_window(HWND(handle.hwnd.get()), ctx).map_err(|error| error.to_string())
    }

    fn for_window(hwnd: HWND, ctx: &egui::Context) -> windows::core::Result<Self> {
        // SAFETY: This object and its apartment live on the GUI thread.
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
        }
        let runtime = WinRt(PhantomData);
        let interop =
            factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        // SAFETY: The caller obtained a live HWND from this native window.
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
        controls.SetIsEnabled(false)?;
        let (tx, commands) = mpsc::sync_channel(16);
        let ctx = ctx.clone();
        let handler = TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, args| {
            if let Some(args) = args
                && let Some(command) = command_for_button(args.Button()?)
                && tx.try_send(command).is_ok()
            {
                ctx.request_repaint();
            }
            Ok(())
        });
        let token = controls.ButtonPressed(&handler)?;
        Ok(Self {
            controls,
            token,
            commands,
            state: None,
            metadata_dirty: false,
            has_metadata: false,
            _runtime: runtime,
        })
    }

    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.commands.try_iter()
    }

    pub fn track_changed(&mut self) {
        self.metadata_dirty = true;
    }

    pub fn refresh(
        &mut self,
        enabled: bool,
        buttons: Buttons,
        metadata: Option<&Media>,
    ) -> windows::core::Result<()> {
        let active = enabled && buttons.can_play;
        let has_track = metadata.is_some();
        let next_state = (active, has_track, buttons);
        if self.state != Some(next_state) {
            let status = if !active {
                MediaPlaybackStatus::Closed
            } else if buttons.playing {
                MediaPlaybackStatus::Playing
            } else if has_track {
                MediaPlaybackStatus::Paused
            } else {
                MediaPlaybackStatus::Stopped
            };
            self.controls.SetIsPlayEnabled(active)?;
            self.controls.SetIsPauseEnabled(active && has_track)?;
            self.controls
                .SetIsPreviousEnabled(active && buttons.previous)?;
            self.controls.SetIsNextEnabled(active && buttons.next)?;
            self.controls.SetPlaybackStatus(status)?;
            self.controls.SetIsEnabled(active)?;
            self.state = Some(next_state);
        }
        let wanted = metadata.filter(|_| active);
        if self.metadata_dirty || wanted.is_some() != self.has_metadata {
            // A failed display update must not disable the transport controls.
            let _ = self.update_metadata(wanted);
            self.metadata_dirty = false;
            self.has_metadata = wanted.is_some();
        }
        Ok(())
    }

    fn update_metadata(&self, metadata: Option<&Media>) -> windows::core::Result<()> {
        let updater = self.controls.DisplayUpdater()?;
        updater.ClearAll()?;
        if let Some(media) = metadata {
            updater.SetType(MediaPlaybackType::Music)?;
            let music = updater.MusicProperties()?;
            music.SetTitle(&HSTRING::from(&media.title))?;
            music.SetArtist(&HSTRING::from(&media.artist))?;
            music.SetAlbumTitle(&HSTRING::from(&media.album))?;
            if let Some(cover) = media.cover.as_deref()
                && let Ok(thumbnail) = cover_reference(cover)
            {
                let _ = updater.SetThumbnail(&thumbnail);
            }
        }
        updater.Update()
    }
}

impl Drop for MediaKeys {
    fn drop(&mut self) {
        let _ = self.controls.SetIsEnabled(false);
        let _ = self.controls.RemoveButtonPressed(self.token);
    }
}

struct WinRt(PhantomData<Rc<()>>);

impl Drop for WinRt {
    fn drop(&mut self) {
        // SAFETY: WinRt is dropped on the thread that initialized the apartment.
        unsafe {
            RoUninitialize();
        }
    }
}

fn cover_reference(data: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
    // SAFETY: The call copies bytes from the live slice into a COM-owned stream.
    let stream = unsafe { SHCreateMemStream(Some(data)) }
        .ok_or_else(|| windows::core::Error::from(E_OUTOFMEMORY))?;
    // SAFETY: The live COM stream is retained by the WinRT adapter.
    let stream: IRandomAccessStream =
        unsafe { CreateRandomAccessStreamOverStream(&stream, BSOS_DEFAULT)? };
    RandomAccessStreamReference::CreateFromStream(&stream)
}

fn command_for_button(button: SystemMediaTransportControlsButton) -> Option<Command> {
    match button {
        SystemMediaTransportControlsButton::Previous => Some(Command::Previous),
        SystemMediaTransportControlsButton::Next => Some(Command::Next),
        SystemMediaTransportControlsButton::Play => Some(Command::Play),
        SystemMediaTransportControlsButton::Pause => Some(Command::Pause),
        _ => None,
    }
}

impl Player {
    pub(super) fn run_transport_commands(&mut self, ctx: &egui::Context) {
        let mut commands = self
            .taskbar
            .as_ref()
            .map(Taskbar::commands)
            .unwrap_or_default();
        if let Ok(media_keys) = &self.media_keys {
            commands.extend(media_keys.commands().filter(|_| self.library.media_keys));
        }
        for command in commands {
            self.run_transport_command(command, ctx);
        }
    }

    fn run_transport_command(&mut self, command: Command, ctx: &egui::Context) {
        let state = self.taskbar_buttons();
        match command {
            Command::Previous if state.previous => self.previous(ctx),
            Command::PlayPause if state.playing => self.pause(),
            Command::PlayPause if state.can_play => self.resume(ctx),
            Command::Play if state.can_play && !state.playing => self.resume(ctx),
            Command::Pause if state.playing => self.pause(),
            Command::Next if state.next => self.step(1, ctx),
            Command::Shuffle => self.toggle_shuffle(),
            _ => {}
        }
    }

    pub(super) fn refresh_windows_controls(&mut self) {
        let buttons = self.taskbar_buttons();
        if let Ok(media_keys) = &mut self.media_keys
            && let Err(error) = media_keys.refresh(
                self.library.media_keys,
                buttons,
                self.playing.as_ref().map(|playing| &playing.media),
            )
        {
            self.media_keys = Err(error.to_string());
        }
        if let Some(taskbar) = &mut self.taskbar {
            taskbar.use_focused_media_keys(self.library.media_keys && self.media_keys.is_err());
            taskbar.refresh(buttons);
        }
    }
}
