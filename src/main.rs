mod commands;
mod events;

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
use serenity::model::prelude::command::Command;
use serenity::model::prelude::Activity;
use serenity::prelude::*;
use songbird::SerenityInit;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
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

        let is_prod = env::var("PROD").is_ok();

        let commands = if is_prod {
            Command::set_global_application_commands(&ctx.http, |commands| {
                commands
                    .create_application_command(|command| commands::ping::register(command))
                    .create_application_command(|command| commands::track::register(command))
            })
            .await
        } else {
            GuildId::set_application_commands(&guild_id, &ctx.http, |commands| {
                commands
                    .create_application_command(|command| commands::ping::register(command))
                    .create_application_command(|command| commands::track::register(command))
            })
            .await
        };

        println!(
            "I now have the following {} slash commands: {:#?}",
            if is_prod { "global" } else { "guild" },
            commands
                .unwrap_or(vec![])
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<String>>()
                .join(", ")
        );
    }
}

async fn init_ess(event_patterns: Arc<Mutex<HashMap<u64, Sender<Event>>>>) -> RealtimeClient {
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
        // characters: Some(CharacterSubscription::Ids(vec![5428713425545165425])),
        characters: Some(CharacterSubscription::All),
        worlds: Some(WorldSubscription::All),
        logical_and_characters_with_worlds: Some(false),
        service: Service::Event,
    };

    let mut client = RealtimeClient::new(config);

    client.subscribe(subscription);

    let mut event_receiver = client.connect().await.expect("Could not connect to ESS");

    task::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            handle_event(event, &event_patterns).await
        }
    });

    client
}

async fn handle_event(event: Event, event_patterns: &Arc<Mutex<HashMap<u64, Sender<Event>>>>) {
    let patterns = event_patterns.lock().await;
    let character_ids = vec![
        get_character_id(&event),
        get_attacker_id(&event),
        get_other_id(&event),
    ];
    for id in character_ids {
        if let Some(character_id) = id {
            if let Some(tx) = patterns.get(&character_id) {
                if let Err(why) = tx.send(event.clone()).await {
                    eprintln!("Unable to send event for processing: {:?}", why);
                }
            }
        }
    }
}

fn get_character_id(event: &Event) -> Option<u64> {
    match event {
        Event::PlayerLogin(login) => Some(login.character_id),
        Event::PlayerLogout(logout) => Some(logout.character_id),
        Event::Death(death) => Some(death.character_id),
        Event::VehicleDestroy(vd) => Some(vd.character_id),
        Event::GainExperience(ge) => Some(ge.character_id),
        Event::PlayerFacilityCapture(pfc) => Some(pfc.character_id),
        Event::PlayerFacilityDefend(pfd) => Some(pfd.character_id),
        Event::ItemAdded => None,
        Event::AchievementEarned => None,
        Event::SkillAdded => None,
        Event::BattleRankUp => None,
        _ => None,
    }
}

fn get_attacker_id(event: &Event) -> Option<u64> {
    match event {
        Event::Death(death) => Some(death.attacker_character_id),
        Event::VehicleDestroy(vd) => Some(vd.attacker_character_id),
        _ => None,
    }
}

fn get_other_id(event: &Event) -> Option<u64> {
    match event {
        Event::GainExperience(ge) => Some(ge.other_id),
        _ => None,
    }
}

struct ESSClient;

impl TypeMapKey for ESSClient {
    type Value = RealtimeClient;
}

struct EventPatterns;

impl TypeMapKey for EventPatterns {
    type Value = Arc<Mutex<HashMap<u64, Sender<Event>>>>;
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
        // let data = client.data.read().await;

        // let songbird_manager = data
        //     .get::<SongbirdKey>()
        //     .cloned()
        //     .expect("Unable to get songbird manager");

        // Init the ESS and start handling events
        task::spawn(async { init_ess(event_patterns).await })
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
