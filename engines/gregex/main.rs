use gregex::Regex;
use lexopt::ValueExt;
use std::io::Write;

use {anyhow::Context, bstr::ByteSlice, lexopt::Arg};

const ENGINES: &[&str] = &["pike_jit", "pike_vm"];

#[derive(Clone, Copy)]
enum Engine {
    PikeJIT,
    PikeVM,
}

impl Engine {
    fn from_str(s: &str) -> Self {
        match s {
            "pike_jit" => Engine::PikeJIT,
            "pike_vm" => Engine::PikeVM,
            _ => unreachable!(),
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let mut p = lexopt::Parser::from_env();

    let engine = match p.next()? {
        None => anyhow::bail!("missing engine name"),
        Some(Arg::Value(v)) => v.string().context("<engine>")?,
        Some(arg) => {
            return Err(
                anyhow::Error::from(arg.unexpected()).context("<engine>")
            );
        }
    };
    anyhow::ensure!(
        ENGINES.contains(&&*engine),
        "unrecognized engine '{}'",
        engine,
    );

    let engine = Engine::from_str(&engine);

    let (mut quiet, mut version) = (false, false);
    while let Some(arg) = p.next()? {
        match arg {
            Arg::Short('h') | Arg::Long("help") => {
                anyhow::bail!("main [--version | --quiet]")
            }
            Arg::Short('q') | Arg::Long("quiet") => {
                quiet = true;
            }
            Arg::Long("version") => {
                version = true;
            }
            _ => return Err(arg.unexpected().into()),
        }
    }
    if version {
        writeln!(std::io::stdout(), "{}", env!("CARGO_PKG_VERSION"))?;
        return Ok(());
    }
    let b = klv::Benchmark::read(std::io::stdin())
        .context("failed to read KLV data from <stdin>")?;

    if b.regex.patterns.len() > 1 {
        anyhow::bail!("multiregex are not supported");
    }

    let samples = match b.model.as_str() {
        "compile" => model_compile(&b, engine)?,
        "count" => model_count(&b, &compile(&b, engine)?)?,
        "count-spans" => model_count_spans(&b, &compile(&b, engine)?)?,
        "count-captures" => model_count_captures(&b, &compile(&b, engine)?)?,
        "grep" => model_grep(&b, &compile(&b, engine)?)?,
        "grep-captures" => model_grep_captures(&b, &compile(&b, engine)?)?,
        // "regex-redux" => model_regex_redux(&b)?,
        _ => anyhow::bail!("unrecognized benchmark model '{}'", b.model),
    };
    if !quiet {
        let mut stdout = std::io::stdout().lock();
        for s in samples.iter() {
            writeln!(stdout, "{},{}", s.duration.as_nanos(), s.count)?;
        }
    }
    Ok(())
}

fn model_compile(
    b: &klv::Benchmark,
    engine: Engine,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run_and_count(
        b,
        |re: Regex| Ok(re.find_all(haystack).count()),
        || compile(b, engine),
    )
}

fn model_count(
    b: &klv::Benchmark,
    re: &Regex,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run(b, || Ok(re.find_all(haystack).count()))
}

fn model_count_spans(
    b: &klv::Benchmark,
    re: &Regex,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run(b, || Ok(re.find_all(haystack).map(|m| m.as_str().len()).sum()))
}

fn model_count_captures(
    b: &klv::Benchmark,
    re: &Regex,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run(b, || {
        let mut count = 0;
        for m in re.find_all_captures(haystack) {
            // Could build an iterator
            for i in 0..m.group_len() {
                if m.get(i).is_some() {
                    count += 1;
                }
            }
        }
        Ok(count)
    })
}

fn model_grep(
    b: &klv::Benchmark,
    re: &Regex,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run(b, || {
        let mut count = 0;
        for line in haystack.lines() {
            if re.is_match(line) {
                count += 1;
            }
        }
        Ok(count)
    })
}

fn model_grep_captures(
    b: &klv::Benchmark,
    re: &Regex,
) -> anyhow::Result<Vec<timer::Sample>> {
    let haystack = b.haystack.to_str()?;
    timer::run(b, || {
        let mut count = 0;
        for line in haystack.lines() {
            for m in re.find_all_captures(line) {
                for i in 0..m.group_len() {
                    if m.get(i).is_some() {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    })
}

fn compile(b: &klv::Benchmark, engine: Engine) -> anyhow::Result<Regex> {
    let pattern = b.regex.one()?;
    let need_cg = match b.model.as_str() {
        "compile" => false,
        "count" => false,
        "count-spans" => false,
        "count-captures" => true,
        "grep" => false,
        "grep-captures" => true,
        _ => unreachable!(),
    };

    let builder = gregex::Builder::new(pattern)
        .case_insensitive(b.regex.case_insensitive)
        // We always use unicode because fuck it, the semantic
        // of no-unicode of the rust-regex parser makes 0 sense to me.
        // At some point when I will have rewritten the parser from scratch
        // we will be able to match with non-unicode character classes.
        //.unicode(b.regex.unicode)
        .cg(need_cg);

    match engine {
        Engine::PikeJIT => {
            let re = builder.pike_jit().map_err(anyhow::Error::from_boxed)?;
            Ok(re)
        }
        Engine::PikeVM => {
            let re = builder.pike_vm().map_err(anyhow::Error::from_boxed)?;
            Ok(re)
        }
    }
}
