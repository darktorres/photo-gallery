//! The display images: the pixels the app shows or sends, made from the files. [`maker`] makes the grid's
//! thumbnails and the viewer's images in the thread's own buffers; [`display`] decodes a file once, upright, at the
//! smallest size a caller needs. Both read the EXIF orientation here and hand it to the codec, which reads none.
//! [`player`] plays an animation's frames into a [`player::Sink`] at their delays. [`transcode`] re-encodes a photo
//! as a JPEG for a service that doesn't take its format.

pub mod display;
pub mod maker;
pub mod player;
pub mod transcode;
