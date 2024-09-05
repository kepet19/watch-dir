use std::collections::HashMap;
use std::fs;
use std::io::{self};
use std::os::windows::fs::MetadataExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::ExecutableCommand;
use itertools::Itertools;
use notify::event::CreateKind;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::sleep;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Terminal;

#[derive(Debug, Clone)]
struct FolderStats {
    size: u64,
    file_count: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Shared state for folder stats
    let folder_stats = Arc::new(Mutex::new(HashMap::new()));

    // Channel to communicate between watcher and UI
    let (tx, mut rx) = mpsc::channel(32);

    // Clone folder stats for the watcher
    let folder_stats_watcher = folder_stats.clone();

    let path = std::path::Path::new(".");
    let display_path = path.canonicalize()?;
    let display_path = display_path.display();
    println!("The path is: {display_path}");
    // Start the file watcher
    tokio::spawn(async move {
        watch_for_files(path, tx, folder_stats_watcher).await;
    });

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let title = format!("Folder Stats: {display_path}");
    // Main loop for UI
    loop {
        // Exit on key press 'q'
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }

        // Update the UI
        terminal.draw(|f| {
            let folder_stats = folder_stats.lock().unwrap();

            // Layout the UI
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.area());

            // Prepare the list of directories with stats
            let items: Vec<ListItem> = folder_stats
                .iter()
                .sorted_by(|a, b| b.1.size.cmp(&a.1.size) )
                .map(|(folder, stats)| {
                    ListItem::new(Span::styled(
                        format!(
                            "Files: {} | Size: {} MB | Folder: {}",
                            stats.file_count, stats.size / 1048576, folder
                        ),
                        Style::default().fg(Color::Yellow),
                    ))
                })
                .collect();

            // Create the list widget
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(title.clone()))
                .style(Style::default().fg(Color::White))
                .highlight_style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                );

            // Render the list
            f.render_widget(list, chunks[0]);
        })?;

        // Check for updates from the file watcher
        if let Ok(_) = rx.try_recv() {
            terminal.autoresize().unwrap(); // Force terminal resize after update
        }

        // Sleep briefly to avoid high CPU usage
        sleep(Duration::from_millis(100)).await;
    }

    // Restore the terminal
    disable_raw_mode()?;
    terminal.backend_mut().execute(crossterm::terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

// Watch for file creation in the directory and update the folder stats
async fn watch_for_files(
    watch_path: &std::path::Path,
    tx: mpsc::Sender<()>,
    folder_stats: Arc<Mutex<HashMap<String, FolderStats>>>,
) {
    let (watcher_tx, watcher_rx) = std::sync::mpsc::channel();

    // Create a watcher object
    let mut watcher: RecommendedWatcher = Watcher::new(watcher_tx, Config::default()).unwrap();
    watcher.watch(watch_path, RecursiveMode::Recursive).unwrap();

    loop {
        match watcher_rx.recv() {
            Ok(Ok(event)) => {
                // If a file is created, update the stats
                if notify::event::EventKind::Create(CreateKind::Any) == event.kind {
                    if let Some(path) = event.paths.get(0) {
                        if path.is_file() {
                            update_folder_stats(path, &folder_stats);
                            tx.send(()).await.unwrap(); // Notify the UI to update
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                println!("Inner error: {:?}", e);
            }
            Err(e) => {
                println!("Watch error: {:?}", e);
            }
        }
    }
}

// Update the folder stats when a new file is added
fn update_folder_stats(path: &std::path::Path, folder_stats: &Arc<Mutex<HashMap<String, FolderStats>>>) {
    let folder = path.parent().unwrap().to_str().unwrap().to_string();

    let file_size = fs::metadata(path).unwrap().file_size();

    let mut stats = folder_stats.lock().unwrap();

    let entry = stats.entry(folder.clone()).or_insert(FolderStats {
        size: 0,
        file_count: 0,
    });

    // Update the file count and size
    entry.file_count += 1;
    entry.size += file_size;
}
