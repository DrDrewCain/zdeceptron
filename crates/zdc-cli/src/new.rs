//! `zdc new`: the one command a reader can run before they know anything.
//!
//! #168's argument, which is the whole justification for this file: every
//! wrong first guess in this language is a diagnostic about a construct
//! the reader has not met yet. `state votes is Map of …` earns a lecture
//! about placements; `Text apiKey` earns one about information flow. Those
//! diagnostics are good, and they are still the expensive way to learn
//! that a `state` line has four words before the type. A working starting
//! point is the cheap way, and until now there was none: a program began
//! at a blank file and whatever the reader remembered of the examples.
//!
//! # What it writes, and why it is exactly two files
//!
//! **No manifest.** `hello.zd`'s own comment says it: "no build config, no
//! bundler entry point, no framework import — the file *is* the program."
//! A `zdc.toml` would be the first thing a new reader had to learn and it
//! would be a lie, because nothing in this compiler reads one. The entry
//! file is named on the command line and that is the entire project model.
//!
//! **`main.zd`, not `<name>.zd`.** The directory already carries the
//! project's name. Naming the entry file after it too would mean the path
//! a reader types into `zdc dev` is a thing they have to remember rather
//! than a thing they can guess, and it would put an arbitrary directory
//! name — which the filesystem lets be almost anything — into a source
//! file name. `main.zd` says what the file is for.
//!
//! **`assets/style.css`.** §6.1 argues that `class is "…"` is how a
//! program reaches CSS, and the asset directory is what made that argument
//! true rather than merely sound: everything under `assets/` ships, and
//! its stylesheets are linked *after* the generated one. A scaffold that
//! names two classes and ships the file defining them makes that door
//! visible on the first build instead of on the day someone reads §6.1.
//! It is also, in practice, the first thing anyone wants.
//!
//! # What the program does, and why it is not "Hello, world"
//!
//! A scaffold whose first edit is deleting it has taught nothing and cost
//! a paragraph of reading. So the generated program is the smallest one
//! that is *worth keeping around while you change it*: one signal you
//! type into, one derived from it, and one event handler that writes a
//! third. That is `starting`, `from`, and `on click` — the three things a
//! reader reaches for first, and the three that make the point that there
//! is no dependency array anywhere.
//!
//! It is deliberately all `client`. A scaffold with `durable` state would
//! be a scaffold whose first read is a `Remote of T` the reader has to
//! eliminate with a `when` before anything renders, which is a fine second
//! program and a hostile first one. The comment in the generated file says
//! what changing that one word would cost, so the door is signposted
//! rather than walked through.
//!
//! # What it prints
//!
//! The next command, with the path already in it. `zdc dev <path>` is the
//! loop — compile, serve, watch, reload — and telling someone that is most
//! of this command's value; a scaffold they cannot run is a directory.
//!
//! # What it refuses
//!
//! A directory with anything in it. Losing someone's work to a scaffold is
//! unforgivable, and `zdc new notes` typed in the wrong terminal is not an
//! unusual afternoon. An *empty* directory is accepted, because `mkdir
//! notes && zdc new notes` is a thing people do and the rule is about
//! losing files rather than about the directory existing. Every entry
//! counts, dotfiles included: a `.git` in there means the directory is
//! someone's, and refusing costs them one more command while the other
//! answer costs them a repository.
//!
//! Nothing is written until the check has passed, so a refusal leaves the
//! directory exactly as it was found.

use std::path::{Path, PathBuf};

/// The entry file's name. See the module comment for why it is not the
/// project's name.
const ENTRY: &str = "main.zd";

/// The scaffold's stylesheet, under [`zdc_codegen::assets::ASSET_DIR`].
const STYLESHEET: &str = "style.css";

/// What was written, in the order it should be read.
pub struct Scaffold {
    /// The project directory, as the caller wrote it — so the paths
    /// printed back are ones they can paste rather than ones they have to
    /// rebuild from a canonical form they never typed.
    root: PathBuf,
    name: String,
}

impl Scaffold {
    /// The path to hand `zdc dev`.
    fn entry(&self) -> PathBuf {
        self.root.join(ENTRY)
    }

    fn stylesheet(&self) -> PathBuf {
        self.root
            .join(zdc_codegen::assets::ASSET_DIR)
            .join(STYLESHEET)
    }

    /// What the terminal says afterwards.
    ///
    /// Two files named with a note each, then the command to run. The
    /// notes are there because a file a reader did not write is a file
    /// they have to open to find out about, and one clause each is cheaper
    /// than opening two files.
    pub fn report(&self) -> String {
        let entry = self.entry().display().to_string();
        let stylesheet = self.stylesheet().display().to_string();
        // Aligned on the longest path rather than on a constant, because
        // the paths are the caller's and can be any length.
        let width = entry.chars().count().max(stylesheet.chars().count());
        format!(
            "zdc new · {name}\n\n  \
             {entry:<width$}  one signal, one derived from it, and one event\n  \
             {stylesheet:<width$}  linked after the stylesheet zdc generates\n\n\
             Next: zdc dev {entry}\n      \
             compiles it, serves it on http://127.0.0.1:{port}, and reloads the browser \
             when you save.\n",
            name = self.name,
            port = zdc_dev::DEFAULT_PORT,
        )
    }
}

/// Write a new project at `root`, or say why not.
///
/// The `Err` is a whole sentence rather than an error enum: every one of
/// these is reported once, by the caller, through the same diagnostic
/// renderer a compile error uses, and an enum with one consumer is a
/// second place for the wording to live.
pub fn scaffold(root: &Path) -> Result<Scaffold, String> {
    let name = project_name(root)?;
    occupied(root)?;

    let assets = root.join(zdc_codegen::assets::ASSET_DIR);
    // The asset directory is created first because creating it creates the
    // project directory too, so there is one `create_dir_all` rather than
    // two that could disagree about which one made the root.
    if let Err(error) = std::fs::create_dir_all(&assets) {
        return Err(format!(
            "`{}` could not be created: {error}. `zdc new` writes a directory, so the path above \
             it has to exist and be writable.",
            assets.display()
        ));
    }

    let scaffold = Scaffold {
        root: root.to_path_buf(),
        name,
    };
    write(&scaffold.entry(), &entry_source(&scaffold.name))?;
    write(&scaffold.stylesheet(), STYLESHEET_SOURCE)?;
    Ok(scaffold)
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("`{}` could not be written: {error}.", path.display()))
}

/// The project's name: the last component of the path it was given.
///
/// `zdc new .` has no last component, so the directory is resolved and its
/// real name used — scaffolding into the directory you are standing in is
/// a reasonable thing to ask for, and "`.` is not a name" would be a
/// refusal with no repair behind it.
fn project_name(root: &Path) -> Result<String, String> {
    if let Some(name) = root.file_name().and_then(|name| name.to_str()) {
        return Ok(name.to_string());
    }

    // Only `.`, `..` and a filesystem root reach here, and all three
    // already exist, so resolving them is a lookup rather than a guess.
    let resolved = root.canonicalize().map_err(|error| {
        format!(
            "`{}` names no directory to create: {error}. `zdc new` takes the path of a directory \
             it will make, and the last part of it is the project's name.",
            root.display()
        )
    })?;
    let Some(name) = resolved.file_name().and_then(|name| name.to_str()) else {
        return Err(format!(
            "`{}` resolves to `{}`, which has no name for the project to take. Give `zdc new` a \
             directory to create, such as `zdc new notes`.",
            root.display(),
            resolved.display()
        ));
    };
    Ok(name.to_string())
}

/// Refuse a directory that already holds something.
///
/// The entry is *named* in the refusal. "The directory is not empty" makes
/// the reader run `ls`; naming what stopped it is the difference between a
/// message that ends the task and one that starts a second one. Which
/// entry is arbitrary when there are several, so the count is given too.
fn occupied(root: &Path) -> Result<(), String> {
    let Ok(mut entries) = std::fs::read_dir(root) else {
        // Not a directory that can be listed: either it does not exist,
        // which is the ordinary case, or it is a file, which the next
        // check catches. A permission failure surfaces from
        // `create_dir_all` with the OS's own reason attached, which is a
        // better sentence than one invented here.
        if root.is_file() {
            return Err(format!(
                "`{}` is a file, so there is no directory to write a project into. Give `zdc new` \
                 a path that does not exist yet.",
                root.display()
            ));
        }
        return Ok(());
    };

    let Some(first) = entries.by_ref().flatten().next() else {
        return Ok(());
    };
    let found = first.file_name().to_string_lossy().to_string();
    let others = entries.flatten().count();
    let also = match others {
        0 => String::new(),
        1 => " and 1 other entry".to_string(),
        many => format!(" and {many} other entries"),
    };
    Err(format!(
        "`{}` is not empty — it already contains `{found}`{also}, so nothing was written. `zdc \
         new` will not write into a directory that holds anything, because a scaffold that \
         overwrote your work would be a worse command than one that refuses. Name a directory \
         that does not exist yet, or empty this one yourself.",
        root.display()
    ))
}

/// The generated program, with the project's name in its `<title>`.
///
/// The name is escaped rather than validated away: §4.2's `Text` literal
/// has exactly four escapes (`\n`, `\t`, `\"`, `\\`), and two of them are
/// precisely the characters a directory name could carry into a string
/// literal and break it. A name holding some other control character would
/// be refused by the lexer rather than silently mangled, which is the
/// correct failure and not one worth a second check here.
fn entry_source(name: &str) -> String {
    let title = name
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    ENTRY_SOURCE.replace("{title}", &title)
}

/// The scaffold's program.
///
/// Held as one constant with a placeholder rather than assembled from
/// pieces: this is the text a reader sees first, and text that is easy to
/// read here is text that is easy to keep true. That it still compiles is
/// `a_scaffolded_project_checks_and_builds`'s job, which runs the real
/// `zdc check` and `zdc build` over it — a template that drifts out of
/// sync with the language should be a test failure rather than a new
/// reader's first experience.
const ENTRY_SOURCE: &str = r#"# main.zd — the whole program.
#
# There is no build config here, no bundler entry point and no framework
# import, because there is nothing for them to configure: this file is the
# program. `zdc dev main.zd` compiles it, serves it, watches it, and
# reloads the browser when you save.
#
# Three declarations, chosen because they are the three you reach for
# first:
#
#   `starting` declares state you set yourself.
#   `from`     declares state the compiler recomputes when its inputs
#              change. There is no dependency array anywhere: `greeting`
#              re-derives because it reads `name`, and the compiler knows
#              that from the signal graph.
#   `on click` is an event handler. It writes state, and the view follows.
#
# Every signal also says *where it lives*. All three of these are
# `client`, so this program is a browser tab and nothing else. Change
# `clicks` to `durable` and the compiler derives the store, the endpoint
# and the transport for you — and the read becomes `Remote of Whole`, so
# the view has to say what to show while it loads. The network appears in
# the type exactly where the network is.

state name     is client Text  starting "world"
state greeting is client Text  from "Hello, " + name + "."
state clicks   is client Whole starting 0

# `view title is "…"` is the whole of the page metadata syntax, and it is
# what the browser tab reads.
view title is "{title}"
    # `class is "…"` names a rule in `assets/style.css`. Everything under
    # `assets/` is copied into the bundle, and every `.css` file there is
    # linked after the stylesheet the compiler generates — so your rules
    # win over the built-in ones without an `!important`.
    Column class is "page"
        Heading greeting

        # Type here and the heading changes. The input writes `name`,
        # `greeting` re-derives because it reads `name`, and the heading
        # re-renders because it reads `greeting`. None of that is wired up
        # above; the compiler read it off the graph.
        Input name, hint is "your name"

        Row class is "controls"
            Button "count a click"
                on click
                    add 1 to clicks
            Text clicks
"#;

/// The scaffold's stylesheet.
///
/// Two rules, one of each kind on purpose: `.page` adds what the built-in
/// `.zd-col` says nothing about, and `.controls` overrides a property
/// `.zd-row` does set. The second is the one worth having, because it
/// demonstrates the cascade order — no `!important`, just a link that
/// comes second.
const STYLESHEET_SOURCE: &str = r#"/* style.css — this project's own rules.
 *
 * Everything under `assets/` is copied into the bundle unchanged, and
 * every `.css` file here is linked *after* the stylesheet zdc generates.
 * That order is the whole mechanism: these rules win without an
 * `!important`, and a CSS framework dropped in beside this file works the
 * same way — `class is "…"` in main.zd is the only thing that has to name
 * it.
 *
 * The two class names below are the two main.zd writes. Nothing generates
 * them and nothing checks them: a class is a string, and CSS is CSS.
 */

/* Additive: `.zd-col` is a flex column with a gap, and says nothing about
 * measure or margin. */
.page {
  max-width: 34rem;
  margin: 4rem auto;
  padding: 0 1.25rem;
}

/* Overriding: `.zd-row` sets `gap: 0.5rem`, and this comes later in the
 * cascade, so this is the gap. */
.controls {
  gap: 1rem;
}
"#;
