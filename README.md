# Toast

Learning a language is an intense process, and there is an overwhelming amount of tools and resources available online. Toast is a desktop app for learning foreign languages and keeping your study life organized. Toast itself is not an all-in-one study tool, but instead a hub where you can keep track of all of your favorite language-learning resources. Toast offers a few optional lightweight study features, like SRS flashcard decks and rich text notebooks, and combines them with daily planning and external resource tracking, so the things you need to review show up on the right day and the rest stays out of your way. Everything lives on your computer. There are no accounts, no servers, and no data leaving your machine.

## What's Inside

- **Decks**: Study flashcards with images and audio, reviewed on a spaced repetition schedule. Cards you know well come back less often, cards you miss come back sooner. Create 'searchable' cards that ensure you never confuse similar cards while studying, and use Toast's unique 'support' field to have things like example sentences, mnemonics, pronunciation guides, and more on each of your cards. You can also import your favorite existing Anki decks (.apkg files) and map how they should look in Toast.
- **Notebooks**: Take rich notes with formatting like tables, images, and audio recordings to do things like practice writing, keep a journal, or answer practice questions.
- **Plans**: Bring it all together and build daily study plans with todo-list items separated by core language skill (reading, speaking, grammar, etc.), track external resources, and link decks for SRS study. Each morning you get a dashboard of what's due today.
- **Stats**: View retention rates, study streaks, time spent, and charts of your progress over time. Stats persist even when the content it refers to is deleted, so you as the user get to decide what is included in your stats.

The point of Toast is to not force any features on you. Prefer another flashcard resource over Toast's decks? Skip our decks and tag it as an external resource instead. Toast is designed to keep track of all of your favorite language tools and doesn't require that you use ours.

## Searchable Cards and Support

Two deck features work together to keep study sessions clean: searchable cards and the support field.

**Searchable** cards appear in the similar cards panel during study. When a card comes up, Toast searches the deck for other searchable cards that share a term with it and lists them next to the card, so words that look alike or share a meaning can be compared on the spot instead of confused. Matching works on the front and back text of a card: separate terms with commas, and anything in parentheses is ignored. Cards that match the front of the studied card are listed first, followed by cards that only match the back.

**Support** is an optional extra field shown after you flip a card, below the back. It's the place for example sentences, mnemonics, pronunciation guides, and context notes. The similar cards panel only ever matches against a card's front and back, so anything in the support field stays out of it and an example sentence full of common words adds context to its own card without bringing unrelated cards into the panel. If you create a flipped copy of a card, both copies keep the same support.

When importing an Anki deck, you can map any of its fields to support. All fields mapped this way can't be edited after import, but you can always add additional support.

## Download and Updates

Download the latest version here: https://github.com/nickt573/Toast/releases/latest

Scroll down to **Assets** and pick the file for your computer:

| Computer | File |
|---|---|
| Mac* | `Toast_x.x.x_aarch64.dmg` or `Toast_x.x.x_x64.dmg` |
| Windows | `Toast_x.x.x_x64-setup.exe` |
| Linux | `Toast_x.x.x_amd64.AppImage` |

***Mac users:** Toast isn't signed with an Apple certificate yet, so macOS will refuse to open it normally on the first launch. After attempting to launch, you must go to System Settings -> Privacy & Security -> Open Anyway.

Any other files that may be in the list (`.sig`, `.tar.gz`, `latest.json`) can be ignored.

Toast checks for updates when it starts. When a new version is out, it asks if you want to update, installs it, and restarts itself. Nothing to download manually after the first install.

## Running from Source

Want to clone the repo and contribute? Toast is a [Tauri 2](https://v2.tauri.app/) app, so you'll need [Node.js](https://nodejs.org/) and [Rust](https://rustup.rs/) installed (see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform).

```bash
git clone https://github.com/nickt573/Toast.git
cd Toast
npm install

# Run the desktop app in development with hot-reload
npm run app

# Build a release binary
npm run tauri build
```

`npm run app` uses a separate dev identifier, so your development database and media are safe from any installed copy of Toast.

The code for Toast was planned and created by entirely me, with help from Claude Code for frontend design and backend cleanup. A special thanks to Bryana for designing the Toast icon!