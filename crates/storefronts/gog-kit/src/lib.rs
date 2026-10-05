//! Reserved for Obelisk's GOG integration. No authentication or downloads yet.

use obelisk_core::storefront::{StorefrontCapabilities, StorefrontDescriptor, StorefrontId};

pub fn descriptor() -> StorefrontDescriptor {
    StorefrontDescriptor {
        id: StorefrontId::Gog,
        display_name: "GOG",
        capabilities: StorefrontCapabilities::default(),
    }
}
