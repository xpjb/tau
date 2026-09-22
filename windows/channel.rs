//! Install identity. Beta must never touch stable Tau's directories or launcher.
#[cfg(feature = "beta")]
pub const NAME: &str = "Tau Beta";
#[cfg(not(feature = "beta"))]
pub const NAME: &str = "Tau";
#[cfg(feature = "beta")]
pub const EXE: &str = "Tau Beta.exe";
#[cfg(not(feature = "beta"))]
pub const EXE: &str = "Tau.exe";
