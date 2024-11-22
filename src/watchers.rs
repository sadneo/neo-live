use std::sync::mpsc::{self, Sender};
use std::fs;
use std::path::PathBuf;
use notify::{Config, INotifyWatcher, RecursiveMode, Watcher};
use notify::{EventKind, event::ModifyKind};

pub fn watch(path: &PathBuf, sender: Sender<String>) {
    let parent = path.parent().unwrap();
    let (tx, rx) = mpsc::channel();
    let mut watcher = INotifyWatcher::new(tx, Config::default()).unwrap();
    watcher.watch(parent, RecursiveMode::Recursive).unwrap();
    // TODO: handle errors, convert to own error type

    for event in rx {
        let Ok(event) = event else {
            let message = format!("Exiting with {event:?}");
            sender.send(message).unwrap();
            return;
        };
        if !event.paths.contains(path) {
            continue;
        }
        
        // println!("\t{:?}", event.kind);
        match event.kind {
            EventKind::Modify(ModifyKind::Data(_)) => {
                // TODO: diff file, then send back the diff somehow, also check error
                let file = fs::read_to_string(path).unwrap();
                sender.send(file).unwrap();
            }
            _ => continue,
        }
    }
}
