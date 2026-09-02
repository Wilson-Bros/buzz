# Josh's Buzz fork

This checkout is Josh and Daniel's shared, mutable Buzz desktop client. The
shared fork is [Wilson-Bros/buzz](https://github.com/Wilson-Bros/buzz), its
`origin` remote points there, and the official Block repository remains
`upstream`.

## Daily use

There is no daily terminal routine. Open **Buzz Josh** from Spotlight or the
Applications folder and use it like any other app. It stays installed until a
new custom build replaces it.

Use the command below only while actively editing Buzz:

```bash
./josh dev
```

That is the fast edit-run loop. It starts the desktop app without a local relay,
database, or Docker stack. On first launch, connect it to the same Buzz
community used by Josh and Daniel. Messages remain interoperable because this
client still speaks Buzz's Nostr-based wire protocol.

The development app is isolated from the official app through its bundle ID,
keychain service, and `~/.buzz-dev` state. It does not silently copy a private
identity from the official app. Complete onboarding in the custom app instead
of moving key material by hand.

## One-time setup

```bash
./josh setup
```

This downloads Buzz's pinned Hermit toolchain, installs the locked JavaScript
dependencies, creates the ignored local `.env`, and installs the repository's
Git hooks. It is safe to rerun.

## Installable personal build

```bash
./josh install
./josh open
```

`install` produces an ad-hoc-signed `Buzz Josh.app` with a stable custom bundle
identity and copies it to `~/Applications`. Rebuilding does not create a new
account or state directory. The official `/Applications/Buzz.app` is left
alone.

After code changes, use `./josh dev` while iterating, then `./josh check` and
`./josh install` when the change is worth keeping in the daily app.

## Keeping up with official Buzz

This is occasional maintenance, not a daily task. Do it before starting a new
round of changes or when an official Buzz fix is worth pulling in. Commit local
work first, then run:

```bash
./josh sync
```

The command first fast-forwards to any Josh/Daniel changes already on
`origin/main`, merges the latest `upstream/main`, and pushes the result back to
the shared fork. It never rewrites published shared history. If Git stops on a
conflict, resolve it and run the relevant checks before pushing. `./josh
status` shows the working tree and upstream divergence.

Daniel has explicit write access through the Wilson Bros organization. He can
clone `https://github.com/Wilson-Bros/buzz.git`, make a branch, and open a pull
request into the fork's `main` branch. Because the official project is public,
the GitHub fork is public too.

## Safety boundary

- Never place an `nsec`, private key, API key, Apple credential, or recovery
  code in Git, `.env`, scripts, issues, or chat.
- Keep compatibility changes inside the existing Nostr event envelope and
  Buzz event-kind contracts if the client must continue talking to hosted
  relays and unmodified clients.
- UI, local behavior, debugging tools, and agent experience can be changed
  freely without forking the protocol.
- Before changing a subsystem, read its nearest `AGENTS.md`, the relevant
  vision document, and the applicable testing guide.
