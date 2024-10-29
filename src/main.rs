use active_incident::ActiveIncidentHandler;
use consts::GUILD_ID;
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::async_trait;

mod logging;
mod active_incident;
mod consts;
mod utils;

struct Handler {
    active_incident: ActiveIncidentHandler,
}

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Some(component) = interaction.as_message_component() {
            if matches!(component.data.kind, ComponentInteractionDataKind::Button) {
                self.active_incident.handle_button(&ctx, component).await;
                return;
            }
        }

        if let Some(command) = interaction.as_command() {
            self.active_incident.handle_command(&ctx, command).await;
            return;
        }
    }

    async fn ready(&self, ctx: Context, _ready: Ready) {
        GUILD_ID.set_commands(&ctx.http, self.active_incident.get_commands()).await.expect("failed to register commands");
        Command::set_global_commands(&ctx.http, vec![]).await.expect("failed to register global commands");
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let token = dotenv::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    let handler = Handler {
        active_incident: Default::default(),
    };

    let mut client = Client::builder(token, GatewayIntents::empty())
        .event_handler(handler)
        .await
        .expect("Error creating client");

    if let Err(error) = client.start().await {
        tracing::error!(?error, "Got error starting client");
    }
}
