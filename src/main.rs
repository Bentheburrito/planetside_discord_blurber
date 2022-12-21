mod commands;

use auraxis::realtime::event::EventNames;
use auraxis::realtime::subscription::{
    CharacterSubscription, EventSubscription, SubscriptionSettings, WorldSubscription,
};
use auraxis::realtime::Service;
use auraxis::realtime::{
    client::{RealtimeClient, RealtimeClientConfig},
    event::Event,
};
use dotenv::dotenv;
use serenity::async_trait;
use serenity::model::application::interaction::{Interaction, InteractionResponseType};
use serenity::model::gateway::Ready;
use serenity::model::id::GuildId;
use serenity::model::prelude::Activity;
use serenity::prelude::*;
use songbird::{SerenityInit, Songbird, SongbirdKey};
use std::env;
use std::sync::Arc;
use tokio::task;
struct Handler;

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
    event_patterns: Arc<Mutex<Vec<(u64, u64, u64)>>>,
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
        while let Some(event) = event_receiver.recv().await {
            match &event {
                Event::Death(death) => {
                    let patterns = event_patterns.lock().await;
                    for pattern in patterns.iter() {
                        match pattern {
                            &(_, _, char_id)
                                if char_id == death.character_id
                                    && char_id == death.attacker_character_id =>
                            {
                                // Play suicide sound
                            }
                            &(guild_id, _, char_id) if char_id == death.character_id => {
                                // Play died sound
                                if let Some(handler_lock) = manager.get(guild_id) {
                                    let mut handler = handler_lock.lock().await;

                                    let source = match songbird::ffmpeg("/media/storage/Desktop/Coding Stuff/Rust/planetside_discord_blurber/audio/fart.mp4").await {
								        Ok(source) => source,
								        Err(why) => {
								            println!("Err starting source: {:?}", why);
											continue;
								        }
								    };

                                    handler.play_source(source);
                                } else {
                                    println!("Could not play source on event");
                                }
                            }
                            &(_, _, char_id) if char_id == death.character_id => {
                                // Play kill sound
                            }
                            _ => (),
                        }
                    }
                }
                Event::VehicleDestroy(_) => {
                    print!("vehicle destroy");
                }
                Event::PlayerLogin(_) => (),
                Event::PlayerLogout(_) => (),
                _ => {
                    println!("{:?}", &event);
                }
            }
        }
    });

    client
}

struct ESSClient;

impl TypeMapKey for ESSClient {
    type Value = RealtimeClient;
}

struct EventPatterns;

impl TypeMapKey for EventPatterns {
    type Value = Arc<Mutex<Vec<(u64, u64, u64)>>>;
}

#[tokio::main]
async fn main() {
    // load dev environment vars
    dotenv().ok();

    // Build the initial patterns from event patterns file
    let event_patterns = if let Ok(content) = std::fs::read_to_string("./commands/event_patterns") {
        let lines = content.split("\n");

        let mut event_patterns: Vec<(u64, u64, u64)> = vec![];
        for line in lines {
            let mut split_line = line.split(",");
            let guild_id = split_line.next().unwrap().parse::<u64>().unwrap();
            let channel_id = split_line.next().unwrap().parse::<u64>().unwrap();
            let character_id = split_line.next().unwrap().parse::<u64>().unwrap();
            event_patterns.push((guild_id, channel_id, character_id));
        }
        event_patterns
    } else {
        vec![]
    };

    let shared_event_patterns = Arc::new(Mutex::new(event_patterns));
    let client_data_event_patterns = shared_event_patterns.clone();

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

        task::spawn(async { init_ess(shared_event_patterns, songbird_manager).await })
            .await
            .expect("Could not initialize ESS client")
    };
    // Put our ESS client/RealtimeClient and event patterns in the client data.
    {
        let mut data = client.data.write().await;
        data.insert::<ESSClient>(ess_client);
        data.insert::<EventPatterns>(client_data_event_patterns)
    }

    // Finally, start a single shard, and start listening to events.
    //
    // Shards will automatically attempt to reconnect, and will perform
    // exponential backoff until it reconnects.
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
