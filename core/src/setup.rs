//! The first-run checklist on Home: what a new player still has to do before
//! Play works, from checks the launcher already makes. Each step names the
//! Play step that does it, so a `failedAt` in the install report
//! (clientstatus.rs) can be matched to the checklist line the player saw.

use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    /// What the player can do about it.
    pub hint: &'static str,
    /// The Play step that does or checks this (as in the install report).
    pub play_stage: &'static str,
    pub done: bool,
}

/// What the launcher already knows. `None` means it has not been checked yet,
/// which shows as not done rather than guessing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inputs {
    pub game_folder: bool,
    pub game_version_ok: Option<bool>,
    pub skse: bool,
    pub helper_mods: bool,
    pub signed_in: bool,
}

pub fn steps(i: &Inputs) -> Vec<Step> {
    vec![
        Step { id: "folder", title: "Choose your Skyrim folder", hint: "Open Settings and pick the folder with SkyrimSE.exe.", play_stage: "Starting", done: i.game_folder },
        Step { id: "version", title: "Skyrim is the version the server needs", hint: "Press Play and the launcher changes it for you.", play_stage: "Starting", done: i.game_folder && i.game_version_ok == Some(true) },
        Step { id: "skse", title: "SKSE is installed", hint: "Press Play: the launcher installs it.", play_stage: "Installing SKSE", done: i.skse },
        Step { id: "helpers", title: "The launcher's helper mods are installed", hint: "Press Play: the launcher installs them.", play_stage: "Installing the launcher's helper mods", done: i.helper_mods },
        Step { id: "signin", title: "Sign in with Discord", hint: "Press Sign in with Discord.", play_stage: "Getting your game session", done: i.signed_in },
    ]
}

/// The checklist shows while a step is open and the player has never started
/// the game from the launcher.
pub fn show(steps: &[Step], launched_before: bool) -> bool {
    !launched_before && steps.iter().any(|s| !s.done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_player_has_every_step_open_in_play_order() {
        let s = steps(&Inputs::default());
        assert_eq!(s.iter().map(|s| s.id).collect::<Vec<_>>(), ["folder", "version", "skse", "helpers", "signin"]);
        assert!(s.iter().all(|s| !s.done));
        assert!(show(&s, false));
    }

    #[test]
    fn the_version_step_needs_a_folder_and_a_matching_game() {
        let mut i = Inputs { game_version_ok: Some(true), ..Default::default() };
        assert!(!steps(&i)[1].done, "no folder, no version");
        i.game_folder = true;
        assert!(steps(&i)[1].done);
        i.game_version_ok = None;
        assert!(!steps(&i)[1].done, "not checked yet is not done");
    }

    #[test]
    fn it_hides_once_everything_is_done_or_the_game_has_started_before() {
        let all = steps(&Inputs { game_folder: true, game_version_ok: Some(true), skse: true, helper_mods: true, signed_in: true });
        assert!(all.iter().all(|s| s.done) && !show(&all, false));
        assert!(!show(&steps(&Inputs::default()), true));
    }

    #[test]
    fn every_step_names_a_play_stage_the_report_can_carry() {
        // These are the texts play() shows under the Play button.
        let known = ["Starting", "Installing SKSE", "Installing the launcher's helper mods", "Getting your game session"];
        assert!(steps(&Inputs::default()).iter().all(|s| known.contains(&s.play_stage)));
    }
}
