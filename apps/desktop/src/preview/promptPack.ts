// Sample prompt pack for the browser preview. Keys and variables mirror the build's pack so the
// editor shows real categories; the wording is placeholder text, not Kestrel's instructions.
const keys: ReadonlyArray<[string, string]> = [
  ["chat.system", ""], ["computer.system", "access_label attachment_instruction workspace_roots"],
  ["computer.attachment_notice", ""], ["computer.objective_continuation", "continuation objective"],
  ["computer.compaction", "omitted_count tool_results"], ["research.planning.system", "archive_snapshot"],
  ["research.planning.user", "lane_count query"], ["research.system", "depth expedition_instruction harness_version model_label"],
  ["research.question", "lane_context query related_context required_wikipedia"], ["research.required_tool", "required_wikipedia wikipedia_count"],
  ["research.new_source", ""], ["research.synthesis", "baseline evidence expedition_instruction"], ["research.retry", ""],
  ["research.expedition_retry", ""], ["collaboration.story.system", ""], ["collaboration.image_asset.system", ""],
  ["collaboration.image_composition.system", ""], ["collaboration.reference.system", ""], ["collaboration.music_caption.system", ""],
  ["collaboration.music_lyrics.system", ""], ["collaboration.invent_story", ""], ["collaboration.source.develop", "field"],
  ["collaboration.source.continue", "field"], ["collaboration.context.story", ""], ["collaboration.context.story_missing", ""],
  ["collaboration.context.image", ""], ["collaboration.context.image_missing", ""], ["collaboration.context.music", ""],
  ["collaboration.context.music_missing", ""], ["collaboration.asset_metadata", "asset_kind asset_name"], ["image.h3.stillness_suffix", ""],
  ...["story", "image_asset", "image_composition", "reference", "music_caption", "music_lyrics"].flatMap((target) =>
    ["develop", "continue"].map((mode): [string, string] => [`collaboration.final.${target}.${mode}`, ""])),
  ...["ask_user", "list_directory", "read_file", "write_file", "create_directory", "move_path", "copy_file", "read_attachment", "run_program", "list_processes", "open_path"]
    .map((tool): [string, string] => [`computer.tool.${tool}`, ""]),
  ...["search_archive", "search_query", "read_source", "source_ref", "section"].map((tool): [string, string] => [`research.tool.${tool}`, ""]),
];

function sampleText(key: string, variables: string): string {
  const slots = variables.split(" ").filter(Boolean).map((name) => `{{${name}}}`).join(", ");
  return `Preview wording for ${key}. The desktop application shows the real instruction here.${slots ? ` It receives ${slots}.` : ""}`;
}

export const samplePromptPackText = JSON.stringify({
  format: "kestrel.prompt-pack",
  version: 1,
  prompts: Object.fromEntries(keys.map(([key, variables]) => [key, sampleText(key, variables)])),
}, null, 2);
