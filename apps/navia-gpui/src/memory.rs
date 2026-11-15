//! Returns memory to the system: freed heap pages, and decoded images while the window
//! can't be seen.

use std::sync::Arc;

use gpui::{Context, RenderImage, Window};

use crate::shell::Shell;

/// Hands freed heap memory back to the system, off the UI thread. `NAVIA_NO_MEM_TUNING=1` skips it.
/// Uses `malloc_trim` on Linux and `HeapCompact` on Windows, where `trim_working_set` also
/// empties the working set that Task Manager reports.
pub fn return_free_memory(trim_working_set: bool) {
    if std::env::var_os("NAVIA_NO_MEM_TUNING").is_some() {
        return;
    }
    let _ = std::thread::Builder::new().name("navia-trim".into()).spawn(move || imp::trim(trim_working_set));
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    pub fn trim(_working_set: bool) {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe { malloc_trim(0) };
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapCompact};
    use windows_sys::Win32::System::ProcessStatus::K32EmptyWorkingSet;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub fn trim(working_set: bool) {
        unsafe {
            HeapCompact(GetProcessHeap(), 0);
            if working_set {
                K32EmptyWorkingSet(GetCurrentProcess());
            }
        }
    }
}

#[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_os = "windows")))]
mod imp {
    pub fn trim(_working_set: bool) {}
}

impl Shell {
    /// Drops every decoded image that's rebuilt on demand once the window is minimized, hidden or
    /// closed to the tray, and returns the memory. Playback and library state are untouched.
    /// Each cache drops its record of requests too. Otherwise an image released while its request
    /// stays on record would never be drawn again.
    pub fn release_hidden_memory(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let mut images: Vec<Arc<RenderImage>> = self.thumbs.release_images();
        images.extend(self.hero_bakes.release());
        if let Some((_, bake)) = self.dock_bake.take() {
            images.push(bake.background.clone());
            images.push(bake.art_stack.clone());
        }
        self.dock_requested = None;
        if let Some((_, _, _, image)) = self.expanded_bg.take() {
            images.push(image);
        }
        self.expanded_requested = None;
        if let Some((_, _, _, image)) = self.bar_glow.take() {
            images.push(image);
        }
        self.bar_glow_requested = None;
        images.extend(self.lyrics_view.release());
        let bytes: usize = images.iter().map(|image| image_bytes(image)).sum();
        if let Some(window) = window {
            for image in images {
                let _ = window.drop_image(image);
            }
        } else {
            drop(images);
        }
        if std::env::var_os("NAVIA_MEMORY_LOG").is_some() || cfg!(target_os = "windows") {
            eprintln!("navia: hidden, released {} MB of images", bytes / (1024 * 1024));
        }
        return_free_memory(true);
        cx.notify();
    }
}

/// The decoded bytes an image holds, BGRA across every frame.
pub fn image_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count()).map(|frame| {
        let size = image.size(frame);
        size.width.0 as usize * size.height.0 as usize * 4
    }).sum()
}
