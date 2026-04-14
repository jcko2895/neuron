//! Stub adapters for platforms we don't have data for yet.
//!
//! Each stub implements SourceAdapter with name/platform and returns
//! an empty Vec from extract_from_file. This way the README accurately
//! shows all planned platforms and the registry is complete.

use crate::common::CommonRecord;
use std::path::Path;

/// Macro to generate a stub adapter with minimal boilerplate.
macro_rules! stub_adapter {
    ($struct_name:ident, $display_name:expr, $platform_id:expr) => {
        pub struct $struct_name;

        impl $struct_name {
            pub fn new() -> Self {
                Self
            }
        }

        impl super::SourceAdapter for $struct_name {
            fn name(&self) -> &str {
                $display_name
            }

            fn platform(&self) -> &str {
                $platform_id
            }

            fn can_handle_file(&self, _path: &Path) -> bool {
                false
            }

            fn extract_from_file(&self, _path: &Path) -> Result<Vec<CommonRecord>, String> {
                Ok(Vec::new())
            }
        }
    };
}

stub_adapter!(PinterestAdapter, "Pinterest", "pinterest");
stub_adapter!(XAdapter, "X", "x");
stub_adapter!(DiscordAdapter, "Discord", "discord");
stub_adapter!(WhatsAppAdapter, "WhatsApp", "whatsapp");
stub_adapter!(TelegramAdapter, "Telegram", "telegram");
stub_adapter!(SignalAdapter, "Signal", "signal");
stub_adapter!(RedditAdapter, "Reddit", "reddit");
stub_adapter!(LinkedInAdapter, "LinkedIn", "linkedin");
stub_adapter!(TikTokAdapter, "TikTok", "tiktok");
stub_adapter!(TidalAdapter, "Tidal", "tidal");
stub_adapter!(SoundCloudAdapter, "SoundCloud", "soundcloud");
stub_adapter!(SteamAdapter, "Steam", "steam");
stub_adapter!(GitHubAdapter, "GitHub", "github");
stub_adapter!(SlackAdapter, "Slack", "slack");
stub_adapter!(NotionAdapter, "Notion", "notion");
stub_adapter!(AppleHealthAdapter, "Apple Health", "apple_health");
stub_adapter!(FinancialAdapter, "Financial", "financial");
stub_adapter!(AmazonAdapter, "Amazon", "amazon");
stub_adapter!(AppleMusicAdapter, "Apple Music", "apple_music");
stub_adapter!(AmazonMusicAdapter, "Amazon Music", "amazon_music");
