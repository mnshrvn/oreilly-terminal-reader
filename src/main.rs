mod auth;
mod client;
mod parser;
mod reader;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "oreilly-terminal-reader")]
#[command(about = "Read O'Reilly books in your terminal")]
struct Cli {
    /// O'Reilly book URL (e.g., https://learning.oreilly.com/library/view/book-name/ISBN/)
    url: String,

    /// Path to cookies file (JSON or Netscape cookies.txt format).
    /// Export from your browser after logging in to learning.oreilly.com.
    #[arg(short, long)]
    cookies: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let book_id = client::extract_book_id(&cli.url)?;
    eprintln!("Book ID: {}", book_id);

    eprintln!("Authenticating...");
    let http_client = auth::build_authenticated_client(
        cli.cookies.as_deref(),
    )
    .await?;

    eprintln!("Fetching book info...");
    let (title, chapters) = client::fetch_book_info(&http_client, &book_id).await?;
    eprintln!("Book: {} ({} chapters)", title, chapters.len());

    let chapter_list: Vec<(String, usize)> = chapters
        .iter()
        .enumerate()
        .map(|(i, ch)| (ch.title.clone(), i))
        .collect();

    let mut current_chapter = 0;

    loop {
        let chapter = &chapters[current_chapter];
        eprintln!("Loading chapter: {}...", chapter.title);

        let html = client::fetch_chapter_content(&http_client, chapter).await?;
        let lines = parser::html_to_terminal(&html);

        let mut reader_ui = reader::Reader::new(
            lines,
            &chapter.title,
            current_chapter,
            chapters.len(),
        );

        match reader_ui.run()? {
            reader::ReaderAction::Quit => break,
            reader::ReaderAction::NextChapter => {
                if current_chapter + 1 < chapters.len() {
                    current_chapter += 1;
                } else {
                    eprintln!("Already at the last chapter.");
                }
            }
            reader::ReaderAction::PrevChapter => {
                if current_chapter > 0 {
                    current_chapter -= 1;
                } else {
                    eprintln!("Already at the first chapter.");
                }
            }
            reader::ReaderAction::SelectChapter => {
                if let Some(idx) =
                    reader::select_chapter(&chapter_list, current_chapter)?
                {
                    current_chapter = idx;
                }
            }
        }
    }

    Ok(())
}
