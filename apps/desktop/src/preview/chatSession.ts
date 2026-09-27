import type { ChatSession } from "../contracts/index";

// A finished chat with long reasoning and a nested-list answer, so the transcript's page breaks can
// be checked in the browser. The wording is sample text.
const at = "2026-08-28T12:00:00Z";
const reasoningParagraph = (topic: string) =>
  `Need maybe mention ${topic}. Red photons have the least energy in visible light, but heat is not about one photon. A hot object emits red because its thermal energy is enough to produce visible photons, not because red is cold. Keep the answer concise but thorough.`;

export const sampleChatSession = {
  id: "preview-chat-1", title: "Why is red associated with hot", modelId: "preview-model", createdAt: at, updatedAt: at,
  messages: [
    { id: "preview-chat-hello", role: "user", content: "Quick one: what makes a sunset red?", createdAt: at },
    { id: "preview-chat-hello-reply", role: "assistant", content: "Sunlight crosses more air near the horizon, so blue light scatters away and the remaining light looks red and orange.", createdAt: at },
    { id: "preview-chat-question", role: "user", content: "Why is red associated with hot... it is the coldest wavelength of rgb, least energy needed.", createdAt: at },
    {
      id: "preview-chat-answer", role: "assistant", createdAt: at,
      reasoning: ["least energy needed", "coldest wavelength of rgb", "RGB versus thermal", "color temperature", "blackbody peaks", "cultural cues", "incandescence", "Wien's law"]
        .map(reasoningParagraph).join("\n\n"),
      content: [
        "Because \"hot\" in \"red is hot\" is not the same as \"hot\" in \"red photons have low energy.\"",
        "You're mixing two different ideas:",
        "1. **Photon energy**\n   - Red light has the longest wavelength in the visible spectrum.\n   - It has the lowest photon energy in visible light.\n   - Blue and violet photons have higher energy.",
        "2. **Hot objects glowing**\n   - When objects get hot, they emit thermal radiation.\n   - At lower temperatures they emit mostly infrared, which we cannot see.\n   - As they get hotter, the first visible color they emit is dull red.\n   - Red is not the hottest color; blue-white hot objects are hotter.",
        "So \"red hot\" means *hot enough to start glowing visibly*.",
      ].join("\n\n"),
    },
    { id: "preview-chat-follow", role: "user", content: "And blue flames are hotter?", createdAt: at },
    {
      id: "preview-chat-follow-reply", role: "assistant", createdAt: at,
      content: "Usually, yes. A blue gas flame burns with plenty of oxygen and reaches a higher temperature than a sooty yellow flame.\n\n- **Blue:** complete combustion; excited molecules emit blue light.\n- **Yellow:** glowing soot particles, cooler and less complete burning.\n\nThe color comes from different processes, so it is a hint, not a thermometer.",
    },
  ],
} satisfies ChatSession;
