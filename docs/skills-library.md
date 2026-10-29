# Skills library

The Skills page (sidebar footer, or **Browse skills** in the command palette)
manages `~/.zeron/skills`, the shared folder every harness loads natively (see
`crates/harness/src/skills.rs`). It is desktop-only and acts on this device.

## Installed

Lists each folder under the root that holds a `SKILL.md`, one level of
grouping deep. A skill shows its origin: skills.sh (from the provenance file
`.zeron-registry.json`), a linked folder, a pack, or local. Rows reveal the
folder, open `SKILL.md` in the default editor, and remove the skill. The
detail view renders `SKILL.md` without its frontmatter and lists the other
files, each opening in the default editor.

Removal is engine-checked (`skill_library::remove_at`): the path must sit
inside the root. A symlinked skill is unlinked and its target is kept. Pack
skills are refused, because the pack's next update would restore them.

## skills.sh

With fewer than two characters typed, the tab lists the most-installed skills
from the skills.sh home page. Longer queries use `https://skills.sh/api/search`.
Install reads the repository tree from the GitHub API and the files from
`raw.githubusercontent.com`. The engine matches the skill by folder name, or
by the frontmatter `name` when the folder is named differently, then stages
the files beside the library and moves them in once `SKILL.md` parses.
Regular files only, at most 200 files and 5 MB.

## Packs

A pack is a git repository of skills (for example
`michael-denyer/pstack-claude`), handled by `crates/engine/src/skill_packs.rs`.

- **Inspect** clones into `~/.zeron/.packs-staging/<token>`. It finds the
  skills root from the first of these that exists:
  1. the plugins listed in `.claude-plugin/marketplace.json`;
  2. `plugins/*/skills`;
  3. `skills`;
  4. skill folders at the repository's top level.

  It then reports the version, the skills, and the names already taken.
- **Install** moves the checkout to `~/.zeron/packs/<name>` and links each
  skill into `~/.zeron/skills`. Taken names are skipped and recorded, never
  shadowed. The pack record in `~/.zeron/packs/packs.json` keeps:
  - source and clone URL;
  - channel and ref;
  - commit and version;
  - skills roots and plugin folders (plugin folders are kept for a later
    `--plugin-dir`);
  - linked and skipped skills;
  - the auto-update flag and the waiting update.
- **Updates** follow the newest release tag (`v1.2.3`, no pre-releases). A
  repository without tags follows its default branch but moves only when
  `VERSION` or the plugin manifest's version changes. A repository with no
  version at all offers new commits as a manual update; auto-update is off
  for it unless the user turns it on. Applying an update checks out the new
  commit and relinks: new skills are added, removed ones are unlinked, the
  rest are kept. The changelog entry for the new version (`CHANGES.md` or
  `CHANGELOG.md`) is shown before updating.
- **Auto-update** is a per-pack toggle. The engine's `skill_packs::Updater`
  first checks two minutes after launch, then every six hours with jitter,
  backing off after failures. The page also checks packs not checked in six
  hours when it opens.
- **Remove pack** unlinks its skills and deletes the checkout.

Nothing from a pack runs. Git is invoked with hooks disabled, no submodules,
no prompts, and the `file` and `ext` transports refused. Install and update
only clone, check out, and link.

## RPC

All methods are local to the device (not relay-forwarded):

- `ListSkillLibrary` returns the library with its packs.
- `ReadLibrarySkill`, `RemoveLibrarySkill`.
- `SearchSkillRegistry`, `PreviewRegistrySkill`, `InstallRegistrySkill`.
- `InspectSkillPack`.
- `InstallSkillPack`, `CheckSkillPackUpdates`, `UpdateSkillPack`,
  `SetSkillPackAutoUpdate` and `RemoveSkillPack` each reply with the fresh
  library.

## Review fixture

`crates/ui/examples/skills-fixture.rs` renders the page against a running
engine and saves captures. Start an engine with a throwaway `HOME`, so its
library is disposable:

```sh
HOME=/tmp/skills-home ZERON_DATA_DIR=/tmp/skills-data ZERON_IPC_PORT=27933 \
  ./target/debug/zeron headless &
ZERON_SKILLS_CAPTURE=/tmp/shots [ZERON_SKILLS_PACKS=1] [ZERON_PALETTE_LIGHT=1] \
  cargo run -p zeron-ui --features appshots-fixture --example skills-fixture
```

Network tests run with `cargo test -p zeron-engine skill_ -- --ignored`.
