use auraxis::realtime::event::Death;
use auraxis::realtime::event::GainExperience;

use auraxis::realtime::event::Event;
use auraxis::realtime::event::ItemAdded;
use auraxis::realtime::event::VehicleDestroy;
use rand::rngs::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use serenity::async_trait;
use serenity::http::Http;
use serenity::model::prelude::*;
use serenity::prelude::*;
use songbird::tracks::TrackHandle;
use songbird::EventContext;
use songbird::EventHandler as VoiceEventHandler;
use songbird::Songbird;
use songbird::TrackEvent;
use std::env;
use std::sync::Arc;

use crate::EventPatterns;
use crate::WeaponIds;

// a killing spree ends after this amount of seconds of no kills
const KILLING_SPREE_INTERVAL: i64 = 12;

async fn handle_revive(ge: &GainExperience, char_id: &u64) -> Option<String> {
    if ge.character_id == *char_id {
        Some("revive_teammate".to_string())
    } else if ge.other_id == *char_id {
        Some("get_revived".to_string())
    } else {
        None
    }
}

async fn handle_death(
    death: &Death,
    char_id: &u64,
    spree_count: &mut u16,
    spree_timestamp: &mut u32,
) -> Option<String> {
    if &death.character_id == char_id {
        if death.character_id == death.attacker_character_id {
            Some("suicide".to_string())
        } else {
            Some("death".to_string())
        }
    } else if &death.attacker_character_id == char_id {
        let kill_category =
            if *spree_timestamp > (death.timestamp.timestamp() - KILLING_SPREE_INTERVAL) as u32 {
                *spree_count += 1;
                match (*spree_count + 1, death.is_headshot) {
                    (1, true) => "kill_headshot",
                    (1, _) => "kill",
                    (2, _) => "kill_double",
                    (3, _) => "kill_triple",
                    (4, _) => "kill_quad",
                    _ => "kill_penta",
                }
            } else {
                *spree_count = 1;
                if death.is_headshot {
                    "kill_headshot"
                } else {
                    "kill"
                }
            };

        *spree_timestamp = death.timestamp.timestamp() as u32;
        Some(kill_category.to_string())
    } else {
        None
    }
}

async fn handle_vehicle_destroy(vd: &VehicleDestroy, char_id: &u64) -> Option<String> {
    if &vd.character_id == char_id && vd.character_id == vd.attacker_character_id {
        Some("destroy_own_vehicle".to_string())
    } else if &vd.attacker_character_id == char_id {
        Some("destroy_vehicle".to_string())
    } else {
        None
    }
}

async fn handle_item_added(
    ia: &ItemAdded,
    char_id: &u64,
    logout_handler: OnLogout,
) -> Option<String> {
    if &ia.character_id == char_id {
        let data = logout_handler.data_clone.read().await;
        let weapon_ids = data.get::<WeaponIds>().unwrap();

        if ia.context == "CaptureTheFlag.TakeFlag" {
            Some("ctf_flag_take".to_string())
        } else if ia.context == "GuildBankWithdrawal" && ia.item_id == 6008913 {
            Some("bastion_pull".to_string())
        } else if weapon_ids.contains(&ia.item_id) {
            Some("unlock_weapon".to_string())
        } else {
            Some("unlock_any".to_string())
        }
    } else {
        None
    }
}

pub async fn handle_event(
    event: &Event,
    char_id: &u64,
    guild_id: &u64,
    spree_count: &mut u16,
    spree_timestamp: &mut u32,
    voicepack: &String,
    manager: &Arc<Songbird>,
    logout_handler: OnLogout,
) {
    let maybe_category = match &event {
        // Revive GEs
        Event::GainExperience(ge) => {
            if ge.experience_id == 7 || ge.experience_id == 53 {
                handle_revive(ge, char_id).await
            } else {
                None
            }
        }
        Event::Death(death) => handle_death(death, char_id, spree_count, spree_timestamp).await,
        Event::VehicleDestroy(vd) => handle_vehicle_destroy(vd, char_id).await,
        Event::PlayerLogin(login) if &login.character_id == char_id => Some("login".to_string()),
        Event::PlayerLogout(logout) => {
            if &logout.character_id == char_id {
                if let Some(handle) =
                    play_random_sound("logout", guild_id, voicepack, manager).await
                {
                    let _ =
                        handle.add_event(songbird::Event::Track(TrackEvent::End), logout_handler);
                }
            }
            None
        }
        // Bastion Pull: https://discord.com/channels/251073753759481856/451032574538547201/780538521492389908
        // Currently "unlock_camo" and "enemy_bastion_pull" are not implemented
        Event::ItemAdded(ia) => handle_item_added(ia, char_id, logout_handler).await,
        _ => None,
    };
    if let Some(category) = maybe_category {
        play_random_sound(&category, guild_id, voicepack, manager).await;
    };
}

// Plays a random track from the given category in the VC, returns Option<TrackHandle> if it has successfully started
async fn play_random_sound(
    sound_category: &str,
    guild_id: &u64,
    voicepack: &String,
    manager: &Arc<Songbird>,
) -> Option<TrackHandle> {
    if let Some(handler_lock) = manager.get(*guild_id) {
        let mut handler = handler_lock.lock().await;

        let pwd = env::current_dir().expect("Could not get pwd.");
        let pwd = pwd.display();
        let category_path = format!("{}/voicepacks/{}/{}.txt", pwd, voicepack, sound_category);
        let category_content = std::fs::read_to_string(category_path.clone()).expect(
            format!(
                "Could not read track names from category file: {}",
                category_path
            )
            .as_str(),
        );
        let track_names = category_content.split("\n").filter(|name| *name != "");
        // Track names file could be empty, so do nothing if None
        let mut rng: StdRng = SeedableRng::from_entropy();
        if let Some(random_track_name) = track_names.choose(&mut rng) {
            let random_track_path = format!(
                "{}/voicepacks/{}/tracks/{}",
                pwd, voicepack, random_track_name
            );
            let source = match songbird::ffmpeg(random_track_path).await {
                Ok(source) => source,
                Err(why) => {
                    println!("Err starting source: {:?}", why);
                    return None;
                }
            };
            println!("Enqueueing source now");
            Some(handler.enqueue_source(source))
        } else {
            None
        }
    } else {
        println!("Could not play source on event");
        None
    }
}

pub struct OnLogout {
    pub character_id: u64,
    pub channel_id: ChannelId,
    pub guild_id: u64,
    pub http: Arc<Http>,
    pub char_name: String,
    pub manager: Arc<Songbird>,
    pub data_clone: Arc<RwLock<TypeMap>>,
}

#[async_trait]
impl VoiceEventHandler for OnLogout {
    async fn act(&self, _: &EventContext<'_>) -> Option<songbird::Event> {
        let _ = self
            .channel_id
            .send_message(&self.http, |m| {
                m.content(format!(
                    "Detected logout for {}, disconnecting now.",
                    self.char_name
                ))
            })
            .await;
        let _ = self.manager.leave(self.guild_id).await;
        let data = self.data_clone.write().await;
        let patterns = data
            .get::<EventPatterns>()
            .cloned()
            .expect("Unable to get patterns in /track");
        let mut patterns = patterns.lock().await;
        patterns.remove(&self.character_id);

        None
    }
}
