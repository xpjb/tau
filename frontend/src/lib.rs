pub mod feed;
pub mod store;
pub mod transport;

#[cfg(target_os = "android")]
mod android;
mod app;
pub mod controller;
#[cfg(not(target_os = "android"))]
mod demo;
#[cfg(not(target_os = "android"))]
mod desktop;
mod details;
mod editor;
mod icons;
mod render;
mod scroll;
mod tooltip;
#[cfg(not(target_os = "android"))]
pub use desktop::run;
