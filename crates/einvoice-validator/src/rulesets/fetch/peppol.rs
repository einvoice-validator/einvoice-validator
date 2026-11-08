//! Peppol BIS Billing 3.0, published as Schematron only: its bundle is built as
//! `scripts/fetch-peppol.sh` builds it. The two Schematron files of the release archive
//! (its commit id checked) are compiled with the ISO Schematron skeleton as packaged in
//! ph-schematron (`ph-schematron-maven-plugin:convert`, engine `iso-schematron`), what
//! the reference validators run, by Apache Maven (its pinned distribution, run with
//! strict checksums, a private local repository in the cache and none of the user's
//! Maven settings) on the Java found on the machine. Each compiled stylesheet must have
//! its pinned SHA-256.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::archive::{Tar, gunzip};
use super::assemble::Files;
use super::{FetchOptions, MAX_TAR, check_sha256, work_dir};
use crate::Error;
use crate::rulesets::cache::{RuleSetCache, TOOLS_DIR};
use crate::rulesets::catalogue::{FamilySpec, FetchedRuleSet, PEPPOL_STYLESHEETS};

fn fetch_error(message: impl Into<String>) -> Error {
    Error::Fetch {
        message: message.into(),
    }
}

/// The Schematron files to compile, by stylesheet name, in the order of
/// [`PEPPOL_STYLESHEETS`].
#[derive(Debug)]
pub(crate) struct Schematron(Vec<(&'static str, Vec<u8>)>);

/// The version the line states after `Schematron version ` (`sed 's/.*Schematron
/// version \([0-9][0-9.]*[0-9]\) .*/\1/'`: the last such statement of the line).
fn stated_version(line: &str) -> Option<&str> {
    const KEY: &str = "Schematron version ";
    let mut positions: Vec<usize> = line.match_indices(KEY).map(|(i, _)| i).collect();
    positions.reverse();
    positions.into_iter().find_map(|i| {
        let rest = &line[i + KEY.len()..];
        let run = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let version = &rest[..run];
        (version.len() >= 2
            && version.starts_with(|c: char| c.is_ascii_digit())
            && version.ends_with(|c: char| c.is_ascii_digit())
            && rest[run..].starts_with(' '))
        .then_some(version)
    })
}

/// The two Schematron files of the release archive `bytes`, checked: the archive is the
/// pinned commit, and the CEN rules state the pinned version.
pub(crate) fn schematron(family: &FamilySpec, bytes: &[u8]) -> Result<Schematron, Error> {
    let pin = |name: &str| family.pins.get(name).map_err(Error::internal);
    let (tag, commit, cen) = (pin("tag")?, pin("commit")?, pin("cen")?);
    let tar = gunzip(bytes, MAX_TAR).map_err(|e| fetch_error(format!("Peppol {tag}: {e}")))?;
    let archive = Tar::open(&tar).map_err(|e| fetch_error(format!("Peppol {tag}: {e}")))?;
    match archive.commit_comment.as_deref() {
        Some(recorded) if recorded == commit => {}
        Some(recorded) => {
            return Err(fetch_error(format!(
                "the Peppol {tag} archive is commit {recorded}, expected {commit}"
            )));
        }
        None => {
            return Err(fetch_error(format!(
                "the Peppol {tag} archive records no commit id"
            )));
        }
    }
    let mut files = Vec::new();
    for (name, _) in PEPPOL_STYLESHEETS {
        let path = format!("rules/sch/{name}.sch");
        let content = archive
            .stripped(&path)
            .ok_or_else(|| fetch_error(format!("the Peppol {tag} archive has no {path}")))?;
        files.push((name, content.to_vec()));
    }
    let cen_sch = files
        .iter()
        .find(|(n, _)| *n == "CEN-EN16931-UBL")
        .map(|(_, c)| String::from_utf8_lossy(c).into_owned())
        .unwrap_or_default();
    // "<!--Schematron version 1.3.16 - Last update: ...-->"
    let stated = cen_sch.lines().find_map(stated_version);
    match stated {
        Some(v) if v == cen => {}
        Some(v) => {
            return Err(fetch_error(format!(
                "rules/sch/CEN-EN16931-UBL.sch states Schematron version {v}, expected {cen}"
            )));
        }
        None => {
            return Err(fetch_error(
                "rules/sch/CEN-EN16931-UBL.sch states no Schematron version",
            ));
        }
    }
    Ok(Schematron(files))
}

/// How to find Java, for messages.
const NO_JAVA: &str = "Peppol BIS Billing 3.0 publishes its rules as Schematron only, and \
    building its bundle compiles them with ph-schematron, as the reference validators do: \
    that needs Java (a recent JRE; the project tests with Java 21) and network access to \
    Maven Central";

/// The Java runtime: the one of the options, else `$JAVA_HOME/bin/java`, else `java` on
/// the `PATH`; it must run (`java -version`).
pub(crate) fn find_java(options: &FetchOptions) -> Result<PathBuf, Error> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let candidate = options
        .java
        .clone()
        .or_else(|| {
            std::env::var_os("JAVA_HOME")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join("bin").join(exe))
        })
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join(exe))
                    .find(|p| p.is_file())
            })
        })
        .ok_or_else(|| {
            fetch_error(format!(
                "{NO_JAVA}. No Java found: install one, or set JAVA_HOME"
            ))
        })?;
    match Command::new(&candidate).arg("-version").output() {
        Ok(out) if out.status.success() => Ok(candidate),
        Ok(out) => Err(fetch_error(format!(
            "{NO_JAVA}. {} -version failed ({}): {}",
            candidate.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) => Err(fetch_error(format!(
            "{NO_JAVA}. {} cannot run: {e}",
            candidate.display()
        ))),
    }
}

/// `text` escaped for XML character data.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn path_text(path: &Path) -> Result<String, Error> {
    path.to_str()
        .map(xml_escape)
        .ok_or_else(|| fetch_error(format!("{}: not valid Unicode", path.display())))
}

/// The project Maven runs: the `convert` goal of ph-schematron-maven-plugin on the
/// Schematron files of `input`, into `output` (as `scripts/fetch-peppol.sh` writes it).
fn pom(plugin: &str, input: &str, output: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>local.build</groupId>
  <artifactId>peppol-schematron-compile</artifactId>
  <version>1</version>
  <packaging>pom</packaging>
  <build>
    <plugins>
      <plugin>
        <groupId>com.helger.maven</groupId>
        <artifactId>ph-schematron-maven-plugin</artifactId>
        <version>{plugin}</version>
        <executions>
          <execution>
            <id>convert</id>
            <phase>validate</phase>
            <goals><goal>convert</goal></goals>
            <configuration>
              <schematronDirectory>{input}</schematronDirectory>
              <schematronPattern>*.sch</schematronPattern>
              <xsltDirectory>{output}</xsltDirectory>
              <schematronEngine>iso-schematron</schematronEngine>
            </configuration>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#
    )
}

/// The Maven arguments: batch mode, strict checksums, the native HTTP transport with
/// five retries from five seconds on an I/O error and on HTTP 429 and 5xx, the given
/// settings and project, the `validate` phase.
fn maven_arguments(settings: &str, pom: &str) -> Vec<String> {
    [
        "--batch-mode",
        "--strict-checksums",
        "--no-transfer-progress",
        "--quiet",
        "-Dmaven.resolver.transport=native",
        "-Daether.connector.http.retryHandler.count=5",
        "-Daether.connector.http.retryHandler.interval=5000",
        "-Daether.connector.http.retryHandler.serviceUnavailable=429,500,502,503,504",
        "--settings",
        settings,
        "--file",
        pom,
        "validate",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// The Maven distribution, extracted afresh into the cache's tools: its home, its
/// classworlds launcher, and the local repository next to it.
struct Maven {
    home: PathBuf,
    classworlds: PathBuf,
    repository: PathBuf,
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io { path, source }
}

fn install_maven(cache: &RuleSetCache, maven: &str, archive: &[u8]) -> Result<Maven, Error> {
    let tools = cache.root().join(TOOLS_DIR);
    let home = tools.join(format!("apache-maven-{maven}"));
    let repository = tools.join("m2");
    // Afresh: a distribution left by an interrupted extraction is not trusted.
    if home.exists() {
        std::fs::remove_dir_all(&home).map_err(io_error(&home))?;
    }
    std::fs::create_dir_all(&repository).map_err(io_error(&repository))?;
    let tar = gunzip(archive, MAX_TAR).map_err(|e| fetch_error(format!("Maven {maven}: {e}")))?;
    Tar::open(&tar)
        .and_then(|archive| archive.extract(&tools))
        .map_err(|e| fetch_error(format!("Maven {maven}: {e}")))?;
    let boot = home.join("boot");
    let classworlds = std::fs::read_dir(&boot)
        .map_err(io_error(&boot))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|e| e == "jar")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("plexus-classworlds-"))
        })
        .ok_or_else(|| fetch_error(format!("Maven {maven}: no boot/plexus-classworlds-*.jar")))?;
    Ok(Maven {
        home,
        classworlds,
        repository,
    })
}

/// Runs Maven in `work` (its `pom.xml` and `settings.xml`): what `bin/mvn` runs, without
/// a shell, the classworlds launcher of the distribution, with the options the script
/// gives Maven.
fn run_maven(java: &Path, maven: &Maven, work: &Path) -> Result<(), Error> {
    let mut command = Command::new(java);
    let native = Command::new(java)
        .args(["--enable-native-access=ALL-UNNAMED", "-version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if native {
        command.arg("--enable-native-access=ALL-UNNAMED");
    }
    let home = &maven.home;
    command
        .arg("-classpath")
        .arg(&maven.classworlds)
        .arg(format!(
            "-Dclassworlds.conf={}",
            home.join("bin").join("m2.conf").display()
        ))
        .arg(format!("-Dmaven.home={}", home.display()))
        .arg(format!(
            "-Dlibrary.jansi.path={}",
            home.join("lib").join("jansi-native").display()
        ))
        .arg(format!(
            "-Dmaven.multiModuleProjectDirectory={}",
            work.display()
        ))
        .arg("org.codehaus.plexus.classworlds.launcher.Launcher")
        .args(maven_arguments(
            &work.join("settings.xml").display().to_string(),
            &work.join("pom.xml").display().to_string(),
        ))
        .current_dir(work);
    let out = command
        .output()
        .map_err(|e| fetch_error(format!("{} cannot run: {e}", java.display())))?;
    if out.status.success() {
        return Ok(());
    }
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut tail: Vec<&str> = log.lines().rev().take(40).collect();
    tail.reverse();
    let network = [
        "Could not transfer",
        "Could not resolve",
        "Unknown host",
        "UnknownHostException",
    ]
    .iter()
    .any(|m| log.contains(m));
    Err(fetch_error(format!(
        "Maven failed ({}){}:\n{}",
        out.status,
        if network {
            ": it could not download the Schematron compiler from Maven Central (no network access?)"
        } else {
            ""
        },
        tail.join("\n")
    )))
}

/// Compiles the Schematron in the work directory `work` and returns the files of the
/// bundle, the compiled stylesheets checked.
fn compile(
    family: &FamilySpec,
    members: &[&FetchedRuleSet],
    java: &Path,
    maven: &Maven,
    schematron: &Schematron,
    work: &Path,
) -> Result<Files, Error> {
    let pin = |name: &str| family.pins.get(name).map_err(Error::internal);
    let (input, output) = (work.join("in"), work.join("out"));
    std::fs::create_dir_all(&input).map_err(io_error(&input))?;
    std::fs::create_dir_all(&output).map_err(io_error(&output))?;
    for (name, content) in &schematron.0 {
        let path = input.join(format!("{name}.sch"));
        std::fs::write(&path, content).map_err(io_error(&path))?;
    }
    // Only this repository: the user's ~/.m2/settings.xml (mirrors, proxies,
    // repositories) and ~/.m2/repository are not used.
    let settings = work.join("settings.xml");
    std::fs::write(
        &settings,
        format!(
            "<settings><localRepository>{}</localRepository></settings>\n",
            path_text(&maven.repository)?
        ),
    )
    .map_err(io_error(&settings))?;
    let pom_path = work.join("pom.xml");
    std::fs::write(
        &pom_path,
        pom(pin("plugin")?, &path_text(&input)?, &path_text(&output)?),
    )
    .map_err(io_error(&pom_path))?;
    run_maven(java, maven, work)?;
    let mut files = Files::new();
    for (name, sha) in PEPPOL_STYLESHEETS {
        let path = output.join(format!("{name}.xslt"));
        let bytes =
            std::fs::read(&path).map_err(|_| fetch_error(format!("Maven wrote no {name}.xslt")))?;
        check_sha256(&format!("the compiled {name}.xslt"), &bytes, pin(sha)?)?;
        files.push((format!("xslt/{name}.xslt"), bytes));
    }
    let manifest = members
        .first()
        .map(|r| r.manifest().as_bytes().to_vec())
        .ok_or_else(|| Error::internal("peppol: no rule set"))?;
    files.push(("bundle.toml".to_owned(), manifest));
    Ok(files)
}

/// Builds the Peppol bundle: Maven extracted into the cache's tools, the Schematron
/// compiled in a work directory, the stylesheets checked, the manifest of the
/// catalogue. The work directory is removed, and the tools too once the bundle is
/// built (kept after a failure, for the next attempt).
pub(crate) fn build(
    cache: &RuleSetCache,
    family: &FamilySpec,
    members: &[&FetchedRuleSet],
    java: &Path,
    maven_archive: &[u8],
    schematron: &Schematron,
    options: &FetchOptions,
) -> Result<Files, Error> {
    let pin = |name: &str| family.pins.get(name).map_err(Error::internal);
    let (version, plugin) = (pin("maven")?, pin("plugin")?);
    let maven = install_maven(cache, version, maven_archive)?;
    options.say(&format!(
        "Peppol BIS Billing: compiling the Schematron with ph-schematron-maven-plugin {plugin} (Apache Maven {version}, {})",
        java.display()
    ));
    let work = work_dir(cache, "peppol")?;
    let result = compile(family, members, java, &maven, schematron, &work);
    let _ = std::fs::remove_dir_all(&work);
    if result.is_ok() {
        let _ = std::fs::remove_dir_all(cache.root().join(TOOLS_DIR));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_stated_version_as_sed_does() {
        assert_eq!(
            stated_version("<!--Schematron version 1.3.16 - Last update: 2025-->"),
            Some("1.3.16")
        );
        // The last statement of the line (the greedy `.*`).
        assert_eq!(
            stated_version("Schematron version 1.2 and Schematron version 1.3.4 x"),
            Some("1.3.4")
        );
        // Not followed by a space, or not ending with a digit: none.
        assert_eq!(stated_version("Schematron version 1.3.16"), None);
        assert_eq!(stated_version("Schematron version 1.3. x"), None);
        assert_eq!(stated_version("Schematron version 1 x"), None);
        assert_eq!(
            stated_version("Schematron version 1.3 - Schematron version x "),
            Some("1.3")
        );
    }

    #[test]
    fn the_project_is_the_scripts() {
        let script = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/fetch-peppol.sh"
        ))
        .unwrap();
        // The heredoc of the script, with its variables replaced.
        let start = script.find("cat >\"$work/pom.xml\" <<EOF\n").unwrap();
        let body = &script[start..];
        let body = &body[body.find('\n').unwrap() + 1..];
        let body = &body[..=body.find("\nEOF\n").unwrap()];
        let expected = body
            .replace("$plugin", "PLUGIN")
            .replace("$work/in", "IN")
            .replace("$work/out", "OUT");
        assert_eq!(pom("PLUGIN", "IN", "OUT"), expected);
        // The settings and the arguments.
        assert!(script.contains("<settings><localRepository>$dir/m2</localRepository></settings>"));
        let arguments = maven_arguments("S", "P").join(" ");
        assert!(
            arguments.contains("--batch-mode --strict-checksums --no-transfer-progress --quiet")
        );
        for line in [
            "-Dmaven.resolver.transport=native",
            "-Daether.connector.http.retryHandler.count=5",
            "-Daether.connector.http.retryHandler.interval=5000",
            "'-Daether.connector.http.retryHandler.serviceUnavailable=429,500,502,503,504'",
        ] {
            assert!(script.contains(line), "{line}");
            assert!(arguments.contains(line.trim_matches('\'')), "{line}");
        }
        assert!(script.contains("--batch-mode --strict-checksums --no-transfer-progress --quiet"));
        assert!(
            script.contains("--settings \"$work/settings.xml\" --file \"$work/pom.xml\" validate")
        );
        assert!(arguments.ends_with("--settings S --file P validate"));
    }

    #[test]
    fn escapes_paths_in_the_project() {
        assert_eq!(xml_escape("/a&b/<c>"), "/a&amp;b/&lt;c&gt;");
    }

    #[test]
    fn a_missing_java_is_explained() {
        let options = FetchOptions::new().java("/nonexistent/bin/java");
        let error = find_java(&options).unwrap_err().to_string();
        assert!(error.contains("Schematron only"), "{error}");
        assert!(error.contains("cannot run"), "{error}");
    }
}
