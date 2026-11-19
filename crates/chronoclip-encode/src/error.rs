use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("FFmpeg error: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),
    #[error("{call} failed: {error}")]
    Call {
        call: &'static str,
        error: ffmpeg_next::Error,
    },
    #[error("encoder `{0}` is not available in this FFmpeg build")]
    EncoderUnavailable(&'static str),
    #[error("out of memory in {0}")]
    OutOfMemory(&'static str),
    #[error("no segments to save")]
    NoSegments,
    #[error("the recording has no video stream")]
    NoVideoStream,
    #[error("path is not valid UTF-8: {}", .0.display())]
    NonUtf8Path(PathBuf),
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Turn a negative FFmpeg return code into an error.
pub(crate) fn check(call: &'static str, ret: i32) -> Result<i32, EncodeError> {
    if ret < 0 {
        Err(EncodeError::Call {
            call,
            error: ffmpeg_next::Error::from(ret),
        })
    } else {
        Ok(ret)
    }
}
