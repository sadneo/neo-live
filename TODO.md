### Action Items
- change server so that it accepts clients sending updates
- change client so that it accepts buffer updates from stdin
- change plugin serve so that it spawns server and client
- change plugin connect
### Overview
#### Server
- [x] client pool of TCP connections to clients for broadcast
- [ ] Whenever a client sends an update broadcast it to everyone else
	- [ ] change the broadcast message to be more simple, like a line diff instead of operation diff
#### Client
- [x] TCP connection to server
- [ ] stdin: nvim buffer contents to send over
	- [ ] change this to send over diffs instead 
- [x] stdout: new nvim buffer contents
	- [ ] consider simpler diffs for nvim to update with
#### Plugin serve
- [x] Spawns server for now just use a single file
	- [ ] server can be configured later to show which file they're editing, everything will be relative to the GitHub repository they're in
- [ ] spawns connect after a delay to allow OS to set up TCP listener
- [ ] stdout: reads from client stdout for updates
- [x] stdin: writes buffer contents
#### Plugin connect
- [x] spawns connect
- [x] reads from client stdout for updates
- [ ] writes to client stdin with buffer contents
