use std::{
    sync::Arc
};
use std::path::Path;
use clap::Parser;
use cuid::cuid2;
use log::error;
use mime_guess::Mime;
use s3::{AddressingStyle, Auth, Client as S3Client, Credentials};
use teloxide::{
    sugar::request::{RequestLinkPreviewExt, RequestReplyExt},
    prelude::*,
    net::Download,
    Bot,
    types::{Audio, Document, FileMeta, InlineKeyboardButton, InlineKeyboardMarkup, PhotoSize, Video, VideoNote, Voice}
};
use url::Url;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, env = "BOT_TOKEN")]
    bot_token: String,

    #[arg(long, env = "S3_HOST")]
    s3_host: String,

    #[arg(long, env = "S3_BUCKET")]
    s3_bucket: String,

    #[arg(long, env = "S3_REGION")]
    s3_region: String,

    #[arg(long, env = "S3_ACCESS_KEY")]
    s3_access_key: String,

    #[arg(long, env = "S3_ACCESS_KEY_SECRET")]
    s3_access_key_secret: String,

    #[arg(long, env = "TELEGRAM_API_URL", default_value = "https://api.telegram.org")]
    telegram_api_url: String,
}

struct S3 {
    s3_client: S3Client,
    s3_bucket: String,
    endpoint: String,
}

impl S3 {
    pub fn new(args: &Args) -> Result<Self, s3::Error> {
        let s3_client = S3Client::builder(&args.s3_host)?
            .region(args.s3_region.clone())
            .auth(Auth::Static(Credentials::new(
                args.s3_access_key.clone(),
                args.s3_access_key_secret.clone())?
            ))
            .addressing_style(AddressingStyle::Path)
            .build()?;

        Ok(Self {
            s3_client,
            s3_bucket: args.s3_bucket.clone(),
            endpoint: args.s3_host.clone(),
        })
    }

    pub async fn put(&self, filename: &str, content_type: Mime, bytes: Vec<u8>) -> Result<String, s3::Error> {
        match self.s3_client.objects()
            .put(self.s3_bucket.clone(), filename.clone())
            .content_type(content_type.as_ref())?
            .body_bytes(bytes)
            .send().await {
            Ok(response) => {
                println!("S3 upload etag: {}", response.etag.unwrap_or("<none>".to_string()));
                Ok(format!("{}/{}/{}", self.endpoint, self.s3_bucket, filename.clone()))
            },
            Err(err) => {
                eprintln!("S3 upload error: {:?}", err);
                Err(err)
            },
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    log::info!("Starting roBOT");

    let args = Args::parse();

    let bot = Bot::new(args.bot_token.clone())
        .set_api_url(Url::parse(args.telegram_api_url.as_str())?);

    let s3 = Arc::new(S3::new(&args)?);

    let handler = Update::filter_message()
        .branch(Message::filter_audio().endpoint(|bot, msg, media: Audio, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.file, media.file_name.unwrap_or("a.mp3".into()), s3, false).await
        }))
        .branch(Message::filter_photo().endpoint(|bot, msg, media: Vec<PhotoSize>, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.into_iter().last().unwrap().file, "a.jpg".into(), s3, true).await
        }))
        .branch(Message::filter_video().endpoint(|bot, msg, media: Video, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.file, media.file_name.unwrap_or("a.mp4".into()), s3, false).await
        }))
        .branch(Message::filter_document().endpoint(|bot, msg, media: Document, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.file, media.file_name.unwrap_or("".into()), s3, false).await
        }))
        .branch(Message::filter_voice().endpoint(|bot, msg, media: Voice, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.file, "a.ogg".into(), s3, false).await
        }))
        .branch(Message::filter_video_note().endpoint(|bot, msg, media: VideoNote, s3: Arc<S3>| async move {
            handle_media(bot, msg, media.file, "a.mp4".into(), s3, false).await
        }));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![s3])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_media(bot: Bot, msg: Message, file: FileMeta, filename: String, s3: Arc<S3>, is_photo: bool) -> anyhow::Result<()> {
    let extension = Path::new(&filename).extension().and_then(|e| e.to_str());
    let mut filename = cuid2();
    if let Some(ext) = extension {
        filename.push_str(format!(".{}", ext).as_str());
    }
    let mime = mime_guess::from_path(&filename).first_or_octet_stream();

    let file = bot.get_file(file.id).await?;
    let mut buf: Vec<u8> = vec![];
    bot.download_file(&file.path, &mut buf).await?;

    match s3.put(filename.as_str(), mime, buf).await {
        Ok(url) => {
            let mut message_builder = bot.send_message::<ChatId, String>(msg.chat.id, url.clone().into())
                .disable_link_preview(true);
            if is_photo {
                message_builder = message_builder
                    .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::url(
                            "Google Lens",
                            url::Url::parse(format!("https://lens.google.com/uploadbyurl?url={}", url).as_str())?
                        ),
                        InlineKeyboardButton::url(
                            "Yandex IS",
                            url::Url::parse(format!("https://yandex.ru/images/search?rpt=imageview&url={}", url).as_str())?
                        ),
                        InlineKeyboardButton::url(
                            "SauceNao",
                            url::Url::parse(format!("https://saucenao.com/search.php?url={}", url).as_str())?
                        ),
                    ]]));
            }
            message_builder.reply_to(msg).await?;
        },
        Err(err) => {
            error!("S3 upload failed: {:?}", err);
            bot.send_message::<ChatId, String>(msg.chat.id, format!("S3 upload failed: {}", err))
                .disable_link_preview(true)
                .reply_to(msg)
                .await?;
        }
    }
    Ok(())
}
