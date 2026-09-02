# Josh's Buzz fork

This checkout is Josh's long-lived, mutable Buzz desktop client. It keeps the
official repository as `upstream` and carries local changes on
`codex/josh-buzz`.

## Daily use

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

Commit local work before syncing, then run:

```bash
./josh sync
```

The command fetches `upstream/main` and rebases the current branch onto it. If
Git stops on a conflict, resolve it, run the relevant checks, and continue with
`git rebase --continue`. `./josh status` shows the working tree and the number
of commits on each side of official Buzz.

There is intentionally no `origin` remote yet: creating or choosing Josh's
GitHub fork is a separate account-level action. Until then, the checkout and
local commits are fully usable, but they are not backed up to GitHub.

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
