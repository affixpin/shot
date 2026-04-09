use shotclaw::{CompletedStep, Config};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--pretty" || a == "-p") {
        shotclaw::emit::set_pretty(true);
    }
    // Remove --pretty/-p from args for further parsing
    let args: Vec<String> = args.into_iter().filter(|a| a != "--pretty" && a != "-p").collect();

    let config = Config::load();

    match args.get(1).map(|s| s.as_str()) {
        Some("plan") => {
            let msg = rest(&args, 2);
            match shotclaw::plan(&config, &msg, "", vec![], None).await {
                Ok(_) => {} // plan is already emitted via events
                Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
            }
        }
        Some("execute") => {
            let step = rest(&args, 2);
            match shotclaw::execute_step(&config, &step, &step, &[], None).await {
                Ok(result) => println!("{result}"),
                Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
            }
        }
        Some("supervise") => {
            let request = rest(&args, 2);
            let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
            let inputs: Vec<StepInput> = serde_json::from_str(&stdin).unwrap_or_default();
            let completed: Vec<CompletedStep> = inputs.into_iter()
                .map(|s| CompletedStep { step: s.step, result: s.result })
                .collect();
            match shotclaw::supervise(&config, &request, &completed, None).await {
                Ok(shotclaw::SupervisorDecision::Done(answer)) => println!("DONE: {answer}"),
                Ok(shotclaw::SupervisorDecision::NeedsWork(feedback)) => println!("NEEDS WORK: {feedback}"),
                Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
            }
        }
        Some(msg) if !msg.starts_with('-') => {
            let msg = rest(&args, 1);
            if let Err(e) = shotclaw::run(&config, &msg).await {
                shotclaw::emit::emit_system("error", serde_json::json!({"message": e.to_string()}));
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            // Legacy: shotclaw --message "hello" / shotclaw -m "hello"
            if let Some(i) = args.iter().position(|a| a == "--message" || a == "-m") {
                let msg = args.get(i + 1).cloned().unwrap_or_default();
                if let Err(e) = shotclaw::run(&config, &msg).await {
                    shotclaw::emit::emit_system("error", serde_json::json!({"message": e.to_string()}));
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            } else {
                eprintln!("Usage:");
                eprintln!("  shotclaw \"message\"              Full plan-execute-replan cycle");
                eprintln!("  shotclaw plan \"message\"          Run planner only");
                eprintln!("  shotclaw execute \"step\"          Run executor for one step");
                eprintln!("  shotclaw replan \"request\" < steps.json");
                std::process::exit(1);
            }
        }
    }
}

/// Join all args from `from` onward into a single string.
fn rest(args: &[String], from: usize) -> String {
    args[from..].join(" ")
}

#[derive(serde::Deserialize, Default)]
struct StepInput {
    step: String,
    result: String,
}
