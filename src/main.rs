extern crate dotenv;
mod calendar;

use std::{collections::HashMap, sync::Mutex};

use calendar::{get_sorted_events, parse_promo_name, Promo};
use lazy_static::lazy_static;
use poise::{
    serenity_prelude::{
        self as serenity, ChannelId, Colour, CreateEmbed, EventHandler, Member, ReactionType, Role,
    },
    Event,
};

use chrono::{Days, Local, NaiveDate, Timelike};
use dotenv::dotenv;
use regex::Regex;

lazy_static! {
    static ref ROLE_REGEX: Regex = Regex::new("[1-4]-[A-Z]*-[1-4][1-2]").unwrap();
}

struct Data {
    edt_msgs: Mutex<HashMap<serenity::MessageId, (NaiveDate, Promo)>>,
} // User data, which is stored and accessible in all command invocations
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

fn get_user_groups(ctx: Context<'_>, member: Member) -> Option<Vec<Promo>> {
    let roles = member.roles(ctx);
    if let Some(roles) = roles {
        let roles: Vec<&Role> = roles
            .iter()
            .filter(|r| ROLE_REGEX.is_match(&r.name))
            .collect();

        let mut promos: Vec<Promo> = Vec::new();
        for role in roles {
            let promo = parse_promo_name(&role.name);
            if promo.is_none() {
                continue;
            }

            promos.push(promo.unwrap());
        }

        return Some(promos);
    }

    None
}

async fn make_events_embed(group: Promo, day: NaiveDate) -> Result<CreateEmbed, String> {
    let events = get_sorted_events(day).await;
    if let Err(err) = events.clone() {
        return Err(format!("Error: {:?}", err));
    }

    let events = events.unwrap();
    if events.len() == 0 {
        return Err(format!(
            "There are no events for {} on {}",
            group,
            day.format("%d/%m/%Y")
        ));
    }

    let mut e = CreateEmbed::default();
    e.title(format!("Emploi du temps: {}", group));

    let timestamp = day.and_hms_opt(0, 0, 0).unwrap();
    e.timestamp(timestamp.and_utc().to_rfc3339());
    for evt in events[&group].clone() {
        e.field(
            format!(
                "{} - {}",
                evt.start.format("%H:%M"),
                evt.end.format("%H:%M"),
            ),
            format!(
                "Matière: {}\nType: {:?}\nSalle: {}",
                if evt.summary.contains("eval") || evt.summary.contains("moodle") {
                    format!("{} (Devoir Noté)", evt.lesson)
                } else {
                    evt.lesson
                },
                evt.event_type,
                evt.location
            ),
            false,
        );
    }
    e.color(Colour::FOOYOO);

    Ok(e)
}

/// Affiche l'emploie du temps d'un groupe ou d'un utilisateur
#[poise::command(slash_command, prefix_command)]
async fn edt(
    ctx: Context<'_>,
    #[description = "Utilisateur"] member: Option<serenity::Member>,
    #[description = "Numéro du group (ex: 32)"] group: Option<String>,
) -> Result<(), Error> {
    let _ = ctx.defer().await;

    let date = Local::now().date_naive();

    let promo: Option<Promo> = if let Some(member) = member {
        let groups = get_user_groups(ctx, member);
        if let Some(groups) = groups {
            if groups.len() == 0 {
                None
            } else {
                Some(groups[0].clone())
            }
        } else {
            None
        }
    } else if let Some(group) = group {
        let promo = parse_promo_name(&group);
        promo
    } else {
        let member = ctx.author_member().await.unwrap();
        let groups = get_user_groups(ctx, member.into_owned());

        if let Some(groups) = groups {
            if groups.len() == 0 {
                None
            } else {
                Some(groups[0].clone())
            }
        } else {
            None
        }
    };

    if let Some(promo) = promo {
        let embed_res = make_events_embed(promo.clone(), date).await;
        let reply = if let Ok(embed) = embed_res {
            ctx.send(|m| {
                m.embed(|e| {
                    *e = embed;
                    e
                })
            })
            .await
            .expect("Failed to send message!")
        } else {
            ctx.say(embed_res.unwrap_err())
                .await
                .expect("Failed to send message!")
        };

        if let Ok(msg) = reply.message().await {
            ctx.data()
                .edt_msgs
                .lock()
                .expect("Failed to lock mutex!")
                .insert(msg.id, (date, promo));
            let _ = msg
                .react(
                    &ctx,
                    serenity::model::channel::ReactionType::Unicode("⏪".to_string()),
                )
                .await;

            let _ = msg
                .react(
                    &ctx,
                    serenity::model::channel::ReactionType::Unicode("⏩".to_string()),
                )
                .await;
        }
    } else {
        let _ = ctx.say("Could not find group for user!").await;
        return Ok(());
    }

    Ok(())
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &Event<'_>,
    _framework: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        Event::ReactionAdd { add_reaction } => {
            if add_reaction.user_id.expect("Failed to get user id!") == ctx.cache.current_user_id()
            {
                return Ok(());
            }

            if data
                .edt_msgs
                .lock()
                .expect("Failed to lock mutex!")
                .contains_key(&add_reaction.message_id)
            {
                let mut date = data
                    .edt_msgs
                    .lock()
                    .expect("Failed to lock mutex!")
                    .get(&add_reaction.message_id)
                    .unwrap()
                    .0;

                match add_reaction.emoji {
                    ReactionType::Unicode(ref emoji) => {
                        if emoji == "⏪" {
                            date = date.checked_sub_days(Days::new(1)).unwrap();
                        } else if emoji == "⏩" {
                            date = date.checked_add_days(Days::new(1)).unwrap();
                        } else {
                            return Ok(());
                        }
                    }
                    _ => {}
                }

                let promo = data
                    .edt_msgs
                    .lock()
                    .expect("Failed to lock mutex!")
                    .get(&add_reaction.message_id)
                    .unwrap()
                    .1
                    .clone();

                let embed_res = make_events_embed(promo.clone(), date).await;
                add_reaction
                    .message(&ctx)
                    .await
                    .expect("Failed to get message!")
                    .edit(ctx, |m| {
                        if let Ok(embed) = embed_res.clone() {
                            m.embed(|e| {
                                *e = embed;
                                e
                            });
                            m.content("");
                        } else {
                            m.content(embed_res.unwrap_err());
                            m.set_embeds(Vec::new());
                        }

                        m
                    })
                    .await
                    .expect("Failed to edit message!");

                data.edt_msgs
                    .lock()
                    .expect("Failed to lock mutex!")
                    .insert(add_reaction.message_id, (date, promo));

                add_reaction
                    .delete(ctx)
                    .await
                    .expect("Failed to delete reaction!");
            }
        }
        _ => {}
    }

    Ok(())
}

struct Handler;

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: serenity::Context, ready: serenity::Ready) {
        println!("{} is connected!", ready.user.name);
        ctx.set_activity(serenity::Activity::watching("les emplois du temps!"))
            .await;

        tokio::spawn(async move {
            loop {
                let now = Local::now();
                let next = now
                    .checked_add_days(Days::new(1))
                    .unwrap()
                    .with_hour(7)
                    .unwrap()
                    .with_minute(0)
                    .unwrap();

                let duration = next - now;
                let duration = duration.to_std().unwrap();

                tokio::time::sleep(duration).await;

                let events = get_sorted_events(Local::now().date_naive()).await;
                if let Err(err) = events.clone() {
                    println!("Error: {:?}", err);
                }

                let events = events.unwrap();
                let channel = ChannelId(1157420627901292704);

                for promo in events.keys() {
                    let embed = make_events_embed(promo.clone(), Local::now().date_naive()).await;
                    if let Ok(embed) = embed {
                        let _ = channel
                            .send_message(&ctx, |m| {
                                m.embed(|e| {
                                    *e = embed;
                                    e
                                })
                            })
                            .await;
                    }
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![edt()],
            event_handler: |_ctx, event, _framework, _data| {
                Box::pin(event_handler(_ctx, event, _framework, _data))
            },
            ..Default::default()
        })
        .token(std::env::var("DISCORD_TOKEN").expect("missing DISCORD_TOKEN"))
        .intents(serenity::GatewayIntents::non_privileged())
        .client_settings(|client_builder| client_builder.event_handler(Handler))
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    edt_msgs: Mutex::new(HashMap::new()),
                })
            })
        });

    framework.run().await.unwrap();
    Ok(())
}
