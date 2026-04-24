use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("X11 connect: {0}")]
    X11Connect(#[from] x11rb::errors::ConnectError),
    #[error("X11 connection: {0}")]
    X11Connection(#[from] x11rb::errors::ConnectionError),
    #[error("X11 reply: {0}")]
    X11Reply(#[from] x11rb::errors::ReplyError),
    #[error("X11 reply or id: {0}")]
    X11ReplyOrId(#[from] x11rb::errors::ReplyOrIdError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
