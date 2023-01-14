use serenity::builder::CreateApplicationCommand;
use serenity::model::prelude::interaction::application_command::CommandDataOption;

use crate::CommandResponse;

pub fn run(_options: &[CommandDataOption]) -> CommandResponse {
    CommandResponse::Message("Hey, I'm alive!".to_string())
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command.name("ping").description("Ping the bot")
}
