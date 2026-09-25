# soulseek-rs-lib

The protocol library from [soulseek-rs](https://github.com/michel/soulseek-rs),
on its own, with changes of ours on top.

`master` mirrors upstream as it is. `lib` holds the library crate and nothing
else from upstream's workspace: no command line client, no terminal interface,
no website or packaging.

## Using it

```toml
[dependencies]
soulseek-rs-lib = { git = "https://github.com/cheshirecatgirl/soulseek-rs-lib.git", branch = "lib" }
```

The crate is imported as `soulseek_rs`. See
[soulseek-rs-lib/README.md](soulseek-rs-lib/README.md) for an example and
[docs/protocol-coverage.md](docs/protocol-coverage.md) for which messages it
handles.

## Changes

- Ignore a user. Their searches, browses and queue requests get no answer.
- Cap download and upload rates across all transfers.
- Fetch only the start of a file, to listen before downloading.
- Set the profile text and picture sent to peers. Keep a peer's picture up
  to 4 MB.
- Expose upload slots, the share listing and privilege state to the host.
- Answer queue requests with the protocol's reasons: "File not shared.",
  "Too many files", "Too many megabytes" and "Pending shutdown.". A repeated
  request keeps its place in the queue.
- Add per-person queue limits. Friends are exempt.
- Add folders shared with friends only. Other users do not see them in
  search replies, browse or folder listings, and they are left out of the
  counts sent to the server.
- Refuse a transfer offer for a file that was never requested. Treat the old
  direction 0 download request as a queue request and answer "Queued".
- Pass a refusal's reason on to the failed download.
- Read the friends-only section of browse listings and search replies as
  locked, and read the queue length from search replies.
- Read why a login was refused, including the detail for an invalid name.
- Confirm a password change (server code 142). Read and ignore eight
  obsolete server codes.

## License

MIT, as upstream. See [LICENSE](LICENSE).
