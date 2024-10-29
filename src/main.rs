use std::fmt::Display;

use std::collections::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serenity::builder::*;
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::async_trait;
use logging::LogHelpers;

mod logging;

const UPTIME_CHANNLE: ChannelId = ChannelId::new(874694150283477083);
const GUILD_ID: GuildId = GuildId::new(686968015715172423);
const BLOCKED_ROLE: RoleId = RoleId::new(1300485192062074890);
const LINKED_ROLE: RoleId = RoleId::new(998597119709552680);
const PATRON_ROLE: RoleId = RoleId::new(998018241182044241);
const TRUSTED_ROLE: RoleId = RoleId::new(1299123473343320134);

#[derive(Serialize, Deserialize)]
pub enum IncidentStatus {
    WaitingForInput,
    SendingAlert,
    AlertSent,
    AlertAcknowledged,
    Resolved,
}

impl Display for IncidentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingForInput => write!(f, "Waiting for more reports"),
            Self::SendingAlert => write!(f, "Sending Pager Duty Alert (might be waking someone up)"),
            Self::AlertSent => write!(f, "Alert has been sent"),
            Self::AlertAcknowledged => write!(f, "We've seen the alert"),
            Self::Resolved => write!(f, "Issue marked as resolved"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[repr(usize)]
enum StatusVote {
    EverythingBroken,
    WebsiteNotLoading,
    TunnelsOffline,
    WorksFine,
}

#[derive(Serialize, Deserialize)]
struct ActiveIncident {
    message_id: MessageId,
    message_url: String,
    initial_user: UserId,
    status: IncidentStatus,
    last_message_update: u64,

    linked_users: Vec<UserId>,
    patron_users: Vec<UserId>,
    trusted_users: Vec<UserId>,
    plain_users: Vec<UserId>,

    total_vote_score: i64,

    votes: HashMap<UserId, i64>,
    counts: [u64; 4],
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

impl ActiveIncident {
    pub fn add_user(&mut self, level: UserLevel, user_id: UserId, vote: StatusVote) -> bool {
        let mut score = level.score() as i64;
        score = match vote {
            StatusVote::WorksFine => -score,
            StatusVote::TunnelsOffline => score * 2,
            StatusVote::EverythingBroken => score * 3,
            StatusVote::WebsiteNotLoading => score,
        };

        self.total_vote_score += score;
        if let Some(existing) = self.votes.insert(user_id, score) {
            self.total_vote_score -= existing;
        }

        let users = match level {
            UserLevel::Plain => &mut self.plain_users,
            UserLevel::Blocked | UserLevel::Linked => &mut self.linked_users,
            UserLevel::Patron => &mut self.patron_users,
            UserLevel::Trusted => &mut self.trusted_users,
        };

        if users.contains(&user_id) {
            return false;
        }

        users.push(user_id);
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UserLevel {
    Plain,
    Blocked,
    Linked,
    Patron,
    Trusted,
}

impl UserLevel {
    const fn score(&self) -> u64 {
        match self {
            Self::Plain => 1,
            Self::Blocked => 0,
            Self::Linked => 10,
            Self::Patron => 30,
            Self::Trusted => 200,
        }
    }
}

impl UserLevel {
    fn from_member(member: &Member) -> Self {
        if member.roles.contains(&BLOCKED_ROLE) {
            return UserLevel::Blocked;
        }

        if member.roles.contains(&TRUSTED_ROLE) {
            return UserLevel::Trusted;
        }

        if member.roles.contains(&PATRON_ROLE) {
            return UserLevel::Patron;
        }

        if member.roles.contains(&LINKED_ROLE) {
            return UserLevel::Linked;
        }

        UserLevel::Plain
    }
}

struct Handler {
    active_incident: RwLock<Option<ActiveIncident>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Some(component) = interaction.as_message_component() {
            if matches!(component.data.kind, ComponentInteractionDataKind::Button) {
                self.handle_button(&ctx, component).await;
                return;
            }
        }

        if let Some(command) = interaction.as_command() {
            if command.data.name == "report-downtime" {
                self.report_downtime(&ctx, command).await;
                return;
            }
        }
    }

    async fn ready(&self, ctx: Context, _ready: Ready) {
        let report_downtime = CreateCommand::new("report-downtime")
            .description("Report downtime on playit.gg. Make sure you're not the only one having issues before reporting!");

        let res = GUILD_ID.set_commands(&ctx.http, vec![
            report_downtime
        ]).await.expect("failed to register commands");

        Command::set_global_commands(&ctx.http, vec![]).await.expect("failed to register global commands");

        println!("Command registered\n{:?}", res);
    }
}

impl Handler {
    async fn handle_button(&self, ctx: &Context, interaction: &ComponentInteraction) {
        let mut lock = self.active_incident.write().await;

        let is_for_issue = 'is_for_issue: {
            if let Some(active) = &*lock {
                if interaction.message.id == active.message_id {
                    break 'is_for_issue true;
                }
            }

            false
        };

        let _ = interaction
            .create_response(ctx.http(), CreateInteractionResponse::Acknowledge).await
            .log_error("failed to ask button interaction");

        if !is_for_issue {
            drop(lock);

            let _ = EditMessage::new()
                .content(interaction.message.content.clone())
                .components(vec![])
                .execute(ctx.http(), (interaction.channel_id, interaction.message.id, None)).await
                .log_error("failed to remove old message components");

            return;
        }

        let active = lock.as_mut().unwrap();
        let Some(member) = interaction.member.as_ref().log_error("button press missing member") else { return; };

        let status_vote = match interaction.data.custom_id.as_str() {
            "not-sure" => StatusVote::EverythingBroken,
            "website" => StatusVote::WebsiteNotLoading,
            "tunnels" => StatusVote::TunnelsOffline,
            "no-issues" => StatusVote::WorksFine,
            _ => return,
        };

        active.add_user(UserLevel::from_member(member), member.user.id, status_vote);
    }

    async fn report_downtime(&self, ctx: &Context, interaction: &CommandInteraction) {
        let Ok(user) = GUILD_ID.member(ctx.http(), interaction.user.clone()).await else { return };
        let user_id = interaction.user.id;

        let user_level = UserLevel::from_member(&user);

        if user_level == UserLevel::Blocked {
            let _ = interaction.create_response(
                ctx.http(),
                CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content("You are blocked from making reports"))
            ).await;

            return;
        }

        /* check if incident is active */
        {
            let mut lock = self.active_incident.write().await;
            if let Some(active) = &mut *lock {
                active.add_user(user_level, user_id, StatusVote::EverythingBroken);

                let message = format!("An incident is currently active, see {}", active.message_url);
                drop(lock);

                /* send response, note: lock dropped for async send */
                let _ = interaction.create_response(
                    ctx.http(),
                    CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(message))
                ).await;

                return;
            }
        }

        if user_level == UserLevel::Plain {
            let _ = interaction.create_response(
                ctx.http(),
                CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content("Link your discord account on https://playit.gg/account/settings/account. If the website is down :/, try to get someone with a linked account to make the report."))
            ).await;

            return;
        }

        let link = {
            let mut lock = self.active_incident.write().await;
            if let Some(active) = &mut *lock {
                active.add_user(user_level, user_id, StatusVote::EverythingBroken);

                let msg = format!("Looks like someone beat you to do it, an incident is currently active: {}", active.message_url);
                drop(lock);

                if let Err(error) = interaction.create_response(
                    ctx.http(),
                    CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(msg))
                ).await {
                    tracing::error!(?error, "failed to send response");
                }

                return;
            }

            let status = if user_level == UserLevel::Trusted {
                IncidentStatus::SendingAlert
            } else {
                IncidentStatus::WaitingForInput
            };

            let Ok(uptime_msg) = UPTIME_CHANNLE.send_message(ctx.http(), CreateMessage::new()
                // .content(format!("**Downtime Reported**\n<@&951953747423154196>\n\nInitial Reporter: <@{}>\nStatus: **{}**\n\n**Are you having issues, what's broken?**", user_id, status))
                .content(format!("**Downtime Reported**\n<ping-downtime-placeholder>\n\nInitial Reporter: <@{}>\nStatus: **{}**\n\n**Are you having issues, what's broken?**", user_id, status))
                .components(vec![
                    CreateActionRow::Buttons(vec![
                        CreateButton::new("not-sure")
                        .label("Not sure / Everything?!?!")
                        .emoji(ReactionType::Custom {
                            animated: true,
                            id: EmojiId::new(1299122424700207166),
                            name: None,
                        }),

                        CreateButton::new("website")
                        .label("Website not loading")
                        .emoji(ReactionType::Custom {
                            animated: true,
                            id: EmojiId::new(1299122422154395659),
                            name: None,
                        }),

                        CreateButton::new("tunnels")
                        .label("Tunnels offline")
                        .emoji(ReactionType::Custom {
                            animated: true,
                            id: EmojiId::new(1299122426390515803),
                            name: None,
                        }),

                        CreateButton::new("no-issues")
                        .label("Works fine for me")
                        .emoji(ReactionType::Custom {
                            animated: true,
                            id: EmojiId::new(1299122884517822575),
                            name: None,
                        }),
                    ]),
                ])
            ).await.log_error("Failed to create incident post") else { return };

            let _ = UPTIME_CHANNLE
                .create_thread_from_message(ctx.http(), uptime_msg.id, CreateThread::new("downtime reported")).await
                .log_error("failed to create thread");

            let mut incient = ActiveIncident {
                message_id: uptime_msg.id,
                message_url: uptime_msg.link(),
                status,
                last_message_update: epoch_ms(),
                initial_user: user_id,

                linked_users: vec![],
                patron_users: vec![],
                trusted_users: vec![],
                plain_users: vec![],

                total_vote_score: 0,
                votes: HashMap::new(),
                counts: [0; 4],
            };

            incient.add_user(user_level, user_id, StatusVote::EverythingBroken);

            lock.replace(incient);
            uptime_msg.link()
        };

        let _ = interaction.create_response(
            ctx.http(),
            CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(format!("Incident created: {}", link)))
        ).await.log_error("failed to send response");
    }
}


#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    // Configure the client with your Discord bot token in the environment.
    let token = dotenv::var("DISCORD_TOKEN").expect("Expected a token in the environment");

    let handler = Handler {
        active_incident: RwLock::new(None),
    };

    // Build our client.
    let mut client = Client::builder(token, GatewayIntents::empty())
        .event_handler(handler)
        .await
        .expect("Error creating client");


    // Finally, start a single shard, and start listening to events.
    //
    // Shards will automatically attempt to reconnect, and will perform exponential backoff until
    // it reconnects.
    if let Err(why) = client.start().await {
        println!("Client error: {why:?}");
    }
}
