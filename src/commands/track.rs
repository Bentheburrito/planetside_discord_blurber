use std::{env, fs};

use auraxis::api::client::{ApiClient, ApiClientConfig};
use auraxis::api::{request::FilterType, CensusCollection};
use serenity::builder::CreateApplicationCommand;
use serenity::model::prelude::command::CommandOptionType;
use serenity::model::prelude::interaction::application_command::{
    ApplicationCommandInteraction, CommandDataOption,
};
use serenity::prelude::Context;

use crate::EventPatterns;

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
            let voicepack = value_string.trim_matches('"');

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

            match query.await {
                Ok(response) => {
                    let character_id_str = response
                        .items
                        .first()
                        .unwrap()
                        .get("character_id")
                        .unwrap()
                        .as_str()
                        .unwrap();

                    let character_id = character_id_str.parse::<u64>().unwrap();

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

                    // TODO: Add a check to make sure someone in this guild isn't using the bot already, instead of
                    // just overwriting with insert().

                    patterns
                        .lock()
                        .await
                        .insert(character_id, (guild.id.0, voicepack.to_string()));

                    return format!(
                        "Successfully joined voice channel, listening to events from {} (ID {})",
                        character_name, character_id
                    )
                    .to_string();
                }
                Err(err) => return format!("Could not query the Census: {:?}", err).to_string(),
            }
        }
        _ => "Please provide a character name".to_string(),
    }
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
