Why I'm writing the design doc
List the requirements
Scope on each stage of the project
First for me and then for others
Going over reasons for design decisions
ipv6 stuff, then ro plugin, then crdt and rw plugin, then think about discovery server/p2p and more features

# Context
## Problem Statement
Currently, if you want to do collaborative editing, you have to use closed tools like VSCode Live
Share. This kind of editor lock-in is not appreciated and I think dev tools like code editors should
be more open.

## Motivation
For university students or remote teams doing pair programming / code reviews, and to enable
learning and knowledge-sharing with minimal setup.

## Requirements
- I want real-time collaborative editing with low latency and syncs without any complexity for the
 users.
- P2P networking for minimal cost to me
    - I do not want to maintain servers, this is not a commercial product
    - In the future, it's possible to look into servers for people who want to be able to
    consistently collaborate with each other.
    - This would make it so that all of your files are on one server, no merge conflicts in
    collaboration, and that you can edit the same files at the same time.
- Editor-agnostic support
    - Communication protocol decoupled from frontend/editor

## Non-Functional Requirements
- Extensibility
    - Pluggable support for features like authentication and logging
- Security
    - Encrypt messages between peers
- Cross-platform
    - Linux, macOS, Windows support for backend
- Performance
    - Fast syncing even for large files
- Low dependency footprint
    - No heavyweight runtime or Electron-style dependency

# Architecture
Two processes, backend and frontend editor plugin
Backend maintains a CRDT on the main instance, all other peers communicate with this main peer
Editor plugin communicates CRDT operations to the editor plugin, and updates the text buffer when
it receives updates from peers

Conflicts are resolved via CRDT, should be no issues here with overwriting each other
\*Planning on using yrs (rust translation of yjs) since they're popular

MessagePack to encode all communication. This format should be well supported so that plugins of
all languages can use it, and with good efficiency since many edits may come in at a time.

TCP as transport layer protocol
- Reliable delivery and widely supported, making it easier to use for an MVP
- For lower latency, may look into QUIC in the future

# Goals
1. Editor Plugin (ro)
   editor plugins spawn the neo-live process
   communicate via stdin and stdout
   use tokio
   (send buffer through stdin)
   (receive buffers from stdout)
2. Editor Plugin (rw)
   crdt
   neo-live converts buffer diffs to crdt operations
   applies buffer diffs, plugin doesn't change
3. Additional Features
   alternative networking
   editor plugins
   cursor share
