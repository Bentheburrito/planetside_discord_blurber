mod commands;
mod events;

use auraxis::api::client::{ApiClient, ApiClientConfig};
use auraxis::realtime::subscription::SubscriptionSettings;
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

// When using EditMessage, the slash command's `run()` must call `interaction.deter()`.
pub enum CommandResponse {
    Message(String),
    EditMessage(String),
}

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::ApplicationCommand(command) = interaction {
            let command_response = match command.data.name.as_str() {
                "ping" => commands::ping::run(&command.data.options),
                "track" => commands::track::run(&command, &ctx, &command.data.options).await,
                _ => CommandResponse::Message("not implemented :(".to_string()),
            };

            match command_response {
                CommandResponse::Message(content) => {
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
                CommandResponse::EditMessage(content) => {
                    if let Err(why) = command
                        .edit_original_interaction_response(&ctx.http, |response| {
                            response.content(content)
                        })
                        .await
                    {
                        println!("Cannot edit response to slash command: {}", why);
                    }
                }
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

pub async fn init_ess(event_patterns: Arc<Mutex<HashMap<u64, Sender<Event>>>>) -> RealtimeClient {
    let sid = env::var("SERVICE_ID").expect("Expected a service ID in the environment");

    let config = RealtimeClientConfig {
        service_id: sid,
        ..RealtimeClientConfig::default()
    };

    let subscription = SubscriptionSettings {
        event_names: None,
        characters: None,
        worlds: None, //Some(WorldSubscription::All),
        logical_and_characters_with_worlds: None,
        service: Service::Event,
    };

    let mut client = RealtimeClient::new(config);

    let mut event_receiver = client.connect().await.expect("Could not connect to ESS");

    client
        .subscribe(subscription)
        .await
        .expect("Could not subscribe after connecting!");

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
        Event::ItemAdded(ia) => Some(ia.character_id),
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

// const WEAPON_ID_URL: &str = "https://census.lithafalcon.cc/get/ps2/item?code_factory_name=Weapon&c:show=item_id&c:limit=5000";
async fn get_weapon_ids() -> Vec<u64> {
    // Fetch character
    let sid = env::var("SERVICE_ID").expect("Expected a service ID in the environment");
    let mut client_config = ApiClientConfig::default();
    client_config.service_id = Some(sid);
    client_config.api_url = Some(String::from("https://census.lithafalcon.cc"));
    client_config.environment = Some(String::from("ps2"));

    let client = ApiClient::new(client_config);

    let query = client
        .get("item")
        .limit(5000)
        .show("item_id")
        .filter(
            "code_factory_name",
            auraxis::api::request::FilterType::EqualTo,
            "Weapon",
        )
        .build();

    match query.await {
        Ok(response) => response
            .items
            .iter()
            .map(|val| {
                val.get("item_id")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            })
            .collect::<Vec<u64>>(),
        Err(err) => panic!("Could not query Sanctuary Census for weapon IDs: {}", err),
    }
}

struct WeaponIds;

impl TypeMapKey for WeaponIds {
    type Value = Arc<Vec<u64>>;
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

    // Init the ESS and start handling events
    let ess_client = task::spawn(async { init_ess(event_patterns).await })
        .await
        .expect("Could not initialize ESS client");

    let weapon_ids = get_weapon_ids().await;

    // Put our ESS client/RealtimeClient and event patterns in the client data.
    {
        let mut data = client.data.write().await;
        data.insert::<ESSClient>(ess_client);
        data.insert::<EventPatterns>(data_event_patterns);
        data.insert::<WeaponIds>(Arc::new(weapon_ids));
    }

    // Finally, start a single shard, and start listening to events.
    //
    // Shards will automatically attempt to reconnect, and will perform
    // exponential backoff until it reconnects.
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
