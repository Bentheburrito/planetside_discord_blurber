use std::time::Duration;
use std::{env, fs};

use auraxis::api::client::{ApiClient, ApiClientConfig};
use auraxis::api::{request::FilterType, CensusCollection};
use serenity::builder::CreateApplicationCommand;
use serenity::model::prelude::command::CommandOptionType;
use serenity::model::prelude::interaction::application_command::{
    ApplicationCommandInteraction, CommandDataOption,
};
use serenity::prelude::Context;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::events::{handle_event, OnLogout};
use crate::EventPatterns;

const TIMEOUT_MINS: u8 = 5;

pub async fn run(
    interaction: &ApplicationCommandInteraction,
    ctx: &Context,
    options: &[CommandDataOption],
) -> String {
    let mut options = options.iter();
    match (options.next(), options.next()) {
        (
            Some(&CommandDataOption {
                ref name,
                value: Some(ref value),
                ..
            }),
            Some(&CommandDataOption {
                name: ref name2,
                value: Some(ref voicepack),
                ..
            }),
        ) if name == "character_name" && name2 == "voicepack" => {
            // For some reason, `value` has quotes surrounding it...
            let value_string = value.to_string();
            let character_name = value_string.trim_matches('"');

            let value_string = voicepack.to_string();
            let voicepack = value_string.trim_matches('"').to_string();

            do_run(interaction, ctx, character_name, voicepack).await
        }
        _ => "Please provide a character name".to_string(),
    }
}

async fn do_run(
    interaction: &ApplicationCommandInteraction,
    ctx: &Context,
    character_name: &str,
    voicepack: String,
) -> String {
    let guild_id = match interaction.guild_id {
        Some(guild_id) => guild_id,
        None => return "Command only available in guilds.".to_string(),
    };

    let guild = if let Some(guild) = ctx.cache.guild(guild_id) {
        guild
    } else {
        return "Could not find guild.".to_string();
    };

    let channel_id = guild
        .voice_states
        .get(&interaction.user.id)
        .and_then(|voice_state| voice_state.channel_id);

    let connect_to = match channel_id {
        Some(channel) => channel,
        None => {
            return "Could not find your voice channel.
			Make sure you're connected to a voice channel, and I have permission to join it."
                .to_string();
        }
    };

    let manager = songbird::get(ctx)
        .await
        .expect("Songbird Voice client placed in at initialization.")
        .clone();

    let _handler = manager.join(guild.id, connect_to).await;

    // Fetch character
    let sid = env::var("SERVICE_ID").expect("Expected a service ID in the environment");
    let mut client_config = ApiClientConfig::default();
    client_config.service_id = Some(sid);

    let client = ApiClient::new(client_config);

    let query = client
        .get(CensusCollection::Character)
        .filter("name.first_lower", FilterType::EqualTo, character_name)
        .limit(1)
        .show("character_id")
        .build();

    let character_id = match query.await {
        Ok(response) => {
            let character_id_str = response
                .items
                .first()
                .unwrap()
                .get("character_id")
                .unwrap()
                .as_str()
                .unwrap();

            character_id_str.parse::<u64>().unwrap()
        }
        Err(err) => return format!("Could not query the Census: {:?}", err).to_string(),
    };

    // ****leaving this commented out for now - auraxis-rs doesn't support adding subscriptions after ****
    // ****starting the realtime client, so just subscribing to all character events for now in main.rs****
    // let mut data = ctx.data.write().await;
    // let ess_client = data.get_mut::<ESSClient>().unwrap();
    // ess_client.subscribe(character_subscription(character_id));

    // Add entry to cached patterns
    let data = ctx.data.write().await;

    let patterns = data
        .get::<EventPatterns>()
        .cloned()
        .expect("Unable to get patterns in /track");
    let mut patterns = patterns.lock().await;

    // Make sure someone in this guild isn't using the bot already
    if patterns.contains_key(&character_id) {
        return format!(
            "
				It looks like someone else in this server is currently tracking a character - if they aren't 
				tracking them anymore, ask them to /untrack their character, or wait for the tracking to timeout 
				({} minutes of no events).
				",
            TIMEOUT_MINS
        )
        .to_string();
    }

    let (tx, mut rx) = mpsc::channel(1000);

    let interaction_channel_id = interaction.channel_id.clone();
    let http = ctx.http.clone();
    let char_name = character_name.to_string();
    let data_clone = ctx.data.clone();
    tokio::task::spawn(async move {
        let mut spree_count = 0;
        let mut spree_timestamp = 0;
        let mut is_idle = false;
        while !is_idle {
            let event = timeout(Duration::from_secs(60 * TIMEOUT_MINS as u64), rx.recv()).await;
            if let Err(_) = event {
                is_idle = true;

                let _ = interaction_channel_id
                    .send_message(&http, |m| {
                        m.content(format!(
                            "No events detected for {} after {} minutes, disconnecting now.",
                            char_name, TIMEOUT_MINS
                        ))
                    })
                    .await;
                let _ = manager.leave(guild_id).await;

                let data = data_clone.write().await;
                let patterns = data
                    .get::<EventPatterns>()
                    .cloned()
                    .expect("Unable to get patterns in /track");
                let mut patterns = patterns.lock().await;
                patterns.remove(&character_id);
            } else if let Ok(Some(event)) = event {
                let logout_handler = OnLogout {
                    character_id: character_id.clone(),
                    channel_id: interaction_channel_id.clone(),
                    guild_id: guild_id.0.clone(),
                    http: http.clone(),
                    char_name: char_name.clone(),
                    manager: manager.clone(),
                    data_clone: data_clone.clone(),
                };
                handle_event(
                    &event,
                    &character_id,
                    &guild_id.0,
                    &mut spree_count,
                    &mut spree_timestamp,
                    &voicepack,
                    &manager,
                    logout_handler,
                )
                .await;
            } else {
                // We got Ok(None), which most likely means the player logged out and the tx was closed.
                // So, we should set is_idle = true to end the loop and thus the thread.
                is_idle = true;
            }
        }
    });

    patterns.insert(character_id, tx);

    return format!(
        "Successfully joined voice channel, listening to events from {} (ID {})",
        character_name, character_id
    )
    .to_string();
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command
        .name("track")
        .description("Track a character")
        .create_option(|c| {
            c.name("character_name")
                .description("Specify the character name you would like to track")
                .kind(CommandOptionType::String)
                .min_length(3)
                .required(true)
        })
        .create_option(|c| {
            c.name("voicepack")
                .description("Specify the voicepack you would like to use")
                .kind(CommandOptionType::String)
                .min_length(1)
                .required(true);

            // Read the /voicepacks dir and dynamically create options
            // this is a monstrosity, I know. If only Rust had with statements...
            let pwd = env::current_dir().expect("Could not get pwd.");
            let pwd = pwd.display();

            match fs::read_dir(format!("{}/voicepacks", pwd)) {
                Ok(ls) => {
                    for file in ls {
                        if let Ok(file) = file {
                            let filename = file.file_name();
                            let filename = filename
                                .to_str()
                                .expect("Could not convert filename to str");

                            if let Ok(file_type) = file.file_type() {
                                if file_type.is_dir() && filename != "TEMPLATE" {
                                    c.add_string_choice(filename, filename);
                                }
                            }
                        }
                    }
                }
                Err(why) => {
                    panic!("I could not read the voicepacks dir: {:?}", why);
                }
            }
            c
        })
}

// fn character_subscription(character_id: CharacterID) -> SubscriptionSettings {
//     SubscriptionSettings {
//         event_names: Some(EventSubscription::Ids(vec![
//             EventNames::PlayerLogin,
//             EventNames::PlayerLogout,
//             EventNames::Death,
//             EventNames::VehicleDestroy,
//             EventNames::GainExperience,
//         ])),
//         characters: Some(CharacterSubscription::Ids(vec![character_id])),
//         worlds: Some(WorldSubscription::All),
//         logical_and_characters_with_worlds: Some(true),
//         service: Service::Event,
//     }
// }
