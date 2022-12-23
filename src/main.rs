mod commands;

use auraxis::realtime::event::{EventNames, GainExperience};
use auraxis::realtime::subscription::{
    CharacterSubscription, EventSubscription, SubscriptionSettings, WorldSubscription,
};
use auraxis::realtime::Service;
use auraxis::realtime::{
    client::{RealtimeClient, RealtimeClientConfig},
    event::Event,
};
use dotenv::dotenv;
use rand::rngs::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use serenity::async_trait;
use serenity::model::application::interaction::{Interaction, InteractionResponseType};
use serenity::model::gateway::Ready;
use serenity::model::id::GuildId;
use serenity::model::prelude::Activity;
use serenity::prelude::*;
use songbird::{SerenityInit, Songbird, SongbirdKey};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::task;

struct Handler;

// a killing spree ends after this amount of seconds of no kills
const KILLING_SPREE_INTERVAL: i64 = 12;

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::ApplicationCommand(command) = interaction {
            let content = match command.data.name.as_str() {
                "ping" => commands::ping::run(&command.data.options),
                "track" => commands::track::run(&command, &ctx, &command.data.options).await,
                _ => "not implemented :(".to_string(),
            };

            if let Err(why) = command
                .create_interaction_response(&ctx.http, |response| {
                    response
                        .kind(InteractionResponseType::ChannelMessageWithSource)
                        .interaction_response_data(|message| message.content(content))
                })
                .await
            {
                println!("Cannot respond to slash command: {}", why);
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        println!(
            "Connected under user {}#{}",
            ready.user.name, ready.user.discriminator
        );

        ctx.set_presence(
            Some(Activity::playing("Planetside 2")),
            serenity::model::user::OnlineStatus::Online,
        )
        .await;

        let guild_id = GuildId(
            env::var("GUILD_ID")
                .expect("Expected GUILD_ID in environment")
                .parse()
                .expect("GUILD_ID must be an integer"),
        );

        let commands = GuildId::set_application_commands(&guild_id, &ctx.http, |commands| {
            commands
                .create_application_command(|command| commands::ping::register(command))
                .create_application_command(|command| commands::track::register(command))
        })
        .await;

        println!(
            "I now have the following guild slash commands: {:#?}",
            commands
                .unwrap_or(vec![])
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<String>>()
                .join(", ")
        );

        // let guild_command = Command::create_global_application_command(&ctx.http, |command| {
        //     commands::wonderful_command::register(command)
        // })
        // .await;

        // println!(
        //     "I created the following global slash command: {:#?}",
        //     guild_command
        // );
    }
}

async fn init_ess(
    event_patterns: Arc<Mutex<HashMap<u64, (u64, String)>>>,
    manager: Arc<Songbird>,
) -> RealtimeClient {
    let sid = env::var("SERVICE_ID").expect("Expected a service ID in the environment");

    let config = RealtimeClientConfig {
        service_id: sid,
        ..RealtimeClientConfig::default()
    };

    let subscription = SubscriptionSettings {
        event_names: Some(EventSubscription::Ids(vec![
            EventNames::PlayerLogin,
            EventNames::PlayerLogout,
            EventNames::Death,
            EventNames::VehicleDestroy,
            EventNames::ItemAdded,
            EventNames::GainExperienceId(7),  // Revive
            EventNames::GainExperienceId(53), // Squad Revive
        ])),
        characters: Some(CharacterSubscription::Ids(vec![5428713425545165425])),
        worlds: Some(WorldSubscription::All),
        logical_and_characters_with_worlds: Some(false),
        service: Service::Event,
    };

    let mut client = RealtimeClient::new(config);

    client.subscribe(subscription);

    let mut event_receiver = client.connect().await.expect("Could not connect to ESS");

    task::spawn(async move {
        let mut killing_sprees = HashMap::new();
        while let Some(event) = event_receiver.recv().await {
            handle_event(&event, &event_patterns, &manager, &mut killing_sprees).await
        }
    });

    client
}

async fn handle_event(
    event: &Event,
    event_patterns: &Arc<Mutex<HashMap<u64, (u64, String)>>>,
    manager: &Arc<Songbird>,
    killing_sprees: &mut HashMap<u64, (u16, u32)>,
) {
    match &event {
        // Revive GEs
        Event::GainExperience(GainExperience {
            experience_id: 7 | 53,
            character_id,
            other_id,
            ..
        }) => {
            let patterns = event_patterns.lock().await;
            if let Some(guild_id) = patterns.get(character_id) {
                play_random_sound("revive_teammate", guild_id, manager).await
            } else if let Some(guild_id) = patterns.get(other_id) {
                play_random_sound("get_revived", guild_id, manager).await
            }
        }
        Event::Death(death) => {
            let patterns = event_patterns.lock().await;
            if let Some(guild_id) = patterns.get(&death.character_id) {
                if death.character_id == death.attacker_character_id {
                    play_random_sound("suicide", guild_id, manager).await
                } else {
                    play_random_sound("death", guild_id, manager).await
                }
            } else if let Some(guild_id) = patterns.get(&death.attacker_character_id) {
                let kill_category = match killing_sprees.get(&death.attacker_character_id) {
                    Some((spree_count, timestamp))
                        if *timestamp
                            > (death.timestamp.timestamp() - KILLING_SPREE_INTERVAL) as u32 =>
                    {
                        let spree_count = spree_count + 1;
                        killing_sprees.insert(
                            death.attacker_character_id,
                            (spree_count, death.timestamp.timestamp() as u32),
                        );
                        match (spree_count, death.is_headshot) {
                            (1, true) => "kill_headshot",
                            (1, _) => "kill",
                            (2, _) => "kill_double",
                            (3, _) => "kill_triple",
                            (4, _) => "kill_quad",
                            _ => "kill_penta",
                        }
                    }
                    _ => {
                        killing_sprees.insert(
                            death.attacker_character_id,
                            (1, death.timestamp.timestamp() as u32),
                        );
                        if death.is_headshot {
                            "kill_headshot"
                        } else {
                            "kill"
                        }
                    }
                };
                play_random_sound(kill_category, guild_id, manager).await
            }
        }
        Event::VehicleDestroy(vd) => {
            let patterns = event_patterns.lock().await;
            if let Some(guild_id) = patterns.get(&vd.character_id) {
                if vd.character_id == vd.attacker_character_id {
                    play_random_sound("destroy_own_vehicle", guild_id, manager).await
                } else {
                    play_random_sound("destroy_vehicle", guild_id, manager).await
                }
            }
        }
        Event::PlayerLogin(login) => {
            let patterns = event_patterns.lock().await;
            if let Some(guild_id) = patterns.get(&login.character_id) {
                play_random_sound("login", guild_id, manager).await
            }
        }
        Event::PlayerLogout(logout) => {
            let patterns = event_patterns.lock().await;
            if let Some(guild_id) = patterns.get(&logout.character_id) {
                play_random_sound("logout", guild_id, manager).await
            }
        }
        _ => {
            println!("{:?}", &event);
        } // Bastion Pull: https://discord.com/channels/251073753759481856/451032574538547201/780538521492389908
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
    (guild_id, voicepack): &(u64, String),
    manager: &Arc<Songbird>,
) {
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
                    return;
                }
            };

            handler.play_source(source);
        }
    } else {
        println!("Could not play source on event");
    }
}

struct ESSClient;

impl TypeMapKey for ESSClient {
    type Value = RealtimeClient;
}

struct EventPatterns;

impl TypeMapKey for EventPatterns {
    type Value = Arc<Mutex<HashMap<u64, (u64, String)>>>;
}

#[tokio::main]
async fn main() {
    // load dev environment vars
    dotenv().ok();

    let event_patterns = Arc::new(Mutex::new(HashMap::new()));
    let data_event_patterns = event_patterns.clone();

    // Configure the client with your Discord bot token in the environment.
    let token = env::var("BOT_TOKEN").expect("Expected a token in the environment");

    // Build our client.
    let mut client = Client::builder(
        token,
        GatewayIntents::non_privileged() | GatewayIntents::GUILD_VOICE_STATES,
    )
    .event_handler(Handler)
    .register_songbird()
    .await
    .expect("Error creating client");
    let ess_client = {
        let data = client.data.read().await;

        let songbird_manager = data
            .get::<SongbirdKey>()
            .cloned()
            .expect("Unable to get songbird manager");

        // Init the ESS and start handling events
        task::spawn(async { init_ess(event_patterns, songbird_manager).await })
            .await
            .expect("Could not initialize ESS client")
    };
    // Put our ESS client/RealtimeClient and event patterns in the client data.
    {
        let mut data = client.data.write().await;
        data.insert::<ESSClient>(ess_client);
        data.insert::<EventPatterns>(data_event_patterns)
    }

    // Finally, start a single shard, and start listening to events.
    //
    // Shards will automatically attempt to reconnect, and will perform
    // exponential backoff until it reconnects.
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
