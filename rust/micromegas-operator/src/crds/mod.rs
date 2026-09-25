mod instance;
mod screen;

pub use instance::{
    AuthSpec, MicromegasInstance, MicromegasInstanceSpec, MicromegasInstanceStatus,
    OidcClientCredentialsSpec, SecretKeyRef,
};
pub use screen::{
    ConfigFrom, ConfigMapKeyRef, Screen, ScreenInstanceStatus, ScreenSpec, ScreenStatus,
};

pub const GROUP: &str = "micromegas.info";
