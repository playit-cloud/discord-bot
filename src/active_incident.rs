use std::fmt::Display;

use std::collections::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serenity::builder::*;
use serenity::model::prelude::*;
use serenity::prelude::*;

use crate::logging::LogHelpers;
use crate::consts::*;
use crate::utils::epoch_ms;

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

pub struct ActiveIncidentHandler {
    active: RwLock<Option<ActiveIncident>>,
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

impl Default for ActiveIncidentHandler {
    fn default() -> Self {
        ActiveIncidentHandler {
            active: RwLock::new(None),
        }
    }
}

impl ActiveIncidentHandler {
    pub fn get_commands(&self) -> Vec<CreateCommand> {
        let report_downtime = CreateCommand::new("report-downtime")
            .description("Report downtime on playit.gg. Make sure you're not the only one having issues before reporting!");
        vec![report_downtime]
    }

    pub async fn handle_button(&self, ctx: &Context, interaction: &ComponentInteraction) -> bool {
        let status_vote = match interaction.data.custom_id.as_str() {
            "not-sure" => StatusVote::EverythingBroken,
            "website" => StatusVote::WebsiteNotLoading,
            "tunnels" => StatusVote::TunnelsOffline,
            "no-issues" => StatusVote::WorksFine,
            _ => return false,
        };

        let mut lock = self.active.write().await;

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

            return true;
        }

        let active = lock.as_mut().unwrap();
        let Some(member) = interaction.member.as_ref().log_error("button press missing member") else { return true; };


        active.add_user(UserLevel::from_member(member), member.user.id, status_vote);
        true
    }

    pub async fn handle_command(&self, ctx: &Context, interaction: &CommandInteraction) -> bool {
        if interaction.data.name != "report-downtime" {
            return false;
        }

        let Ok(user) = GUILD_ID.member(ctx.http(), interaction.user.clone()).await else { return true };
        let user_id = interaction.user.id;

        let user_level = UserLevel::from_member(&user);

        if user_level == UserLevel::Blocked {
            let _ = interaction.create_response(
                ctx.http(),
                CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content("You are blocked from making reports"))
            ).await;

            return true;
        }

        /* check if incident is active */
        {
            let mut lock = self.active.write().await;
            if let Some(active) = &mut *lock {
                active.add_user(user_level, user_id, StatusVote::EverythingBroken);

                let message = format!("An incident is currently active, see {}", active.message_url);
                drop(lock);

                /* send response, note: lock dropped for async send */
                let _ = interaction.create_response(
                    ctx.http(),
                    CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(message))
                ).await;

                return true;
            }
        }

        if user_level == UserLevel::Plain {
            let _ = interaction.create_response(
                ctx.http(),
                CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content("Link your discord account on https://playit.gg/account/settings/account. If the website is down :/, try to get someone with a linked account to make the report."))
            ).await;

            return true;
        }

        let link = {
            let mut lock = self.active.write().await;
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

                return true;
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
            ).await.log_error("Failed to create incident post") else { return true };

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

        true
    }
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
