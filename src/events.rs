use auraxis::realtime::event::GainExperience;

use auraxis::realtime::event::Event;
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

// a killing spree ends after this amount of seconds of no kills
const KILLING_SPREE_INTERVAL: i64 = 12;

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
    match &event {
        // Revive GEs
        Event::GainExperience(GainExperience {
            experience_id: 7 | 53,
            character_id,
            other_id,
            ..
        }) => {
            if character_id == char_id {
                play_random_sound("revive_teammate", guild_id, voicepack, manager).await;
            } else if other_id == char_id {
                play_random_sound("get_revived", guild_id, voicepack, manager).await;
            }
        }
        Event::Death(death) => {
            if &death.character_id == char_id {
                if death.character_id == death.attacker_character_id {
                    play_random_sound("suicide", guild_id, voicepack, manager).await;
                } else {
                    play_random_sound("death", guild_id, voicepack, manager).await;
                }
            } else if &death.attacker_character_id == char_id {
                let kill_category = if *spree_timestamp
                    > (death.timestamp.timestamp() - KILLING_SPREE_INTERVAL) as u32
                {
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
                play_random_sound(kill_category, guild_id, voicepack, manager).await;
            }
        }
        Event::VehicleDestroy(vd) => {
            if &vd.character_id == char_id {
                if vd.character_id == vd.attacker_character_id {
                    play_random_sound("destroy_own_vehicle", guild_id, voicepack, manager).await;
                } else {
                    play_random_sound("destroy_vehicle", guild_id, voicepack, manager).await;
                }
            }
        }
        Event::PlayerLogin(login) => {
            if &login.character_id == char_id {
                play_random_sound("login", guild_id, voicepack, manager).await;
            }
        }
        Event::PlayerLogout(logout) => {
            if &logout.character_id == char_id {
                if let Some(handle) =
                    play_random_sound("logout", guild_id, voicepack, manager).await
                {
                    let _ =
                        handle.add_event(songbird::Event::Track(TrackEvent::End), logout_handler);
                }
            }
        }
        _ => (), // Bastion Pull: https://discord.com/channels/251073753759481856/451032574538547201/780538521492389908
                 // Event::ItemAdded => {
                 //     let patterns = event_patterns.lock().await;
                 //     for pattern in patterns.iter() {
                 //         match pattern {
                 //             &(_, _, char_id)
                 //                 if char_id == ia.character_id
                 //                     && ia.context == "GuildBankWithdrawal"
                 //                     && ia.item_id == 6008913 => {}
                 //         }
                 //     }
                 // }
    }
}

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
            let random_track_path =
                format!("{}/voicepacks/crashmore/tracks/{}", pwd, random_track_name);
            let source = match songbird::ffmpeg(random_track_path).await {
                Ok(source) => source,
                Err(why) => {
                    println!("Err starting source: {:?}", why);
                    return None;
                }
            };
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
        println!("detected end of logout track!");
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
        println!("Should have removed the pattern on logout now");
        None
    }
}
