// Headless equivalent of the Image desk's explicit Apply proposal step.
// Vite loads the actual UI parser, so acceptance does not substitute a permissive parser.
import { readFileSync, writeFileSync } from "node:fs";
import { imageCompositionPreview, parseImageProposal } from "../apps/desktop/src/features/studio/image/ImageStudio";
import { sampleImage } from "../apps/desktop/src/preview/studioFixtures";

const [input, output, previewOutput] = process.argv.slice(2);
if (!input || !output) throw new Error("Usage: vite-node scripts/image-acceptance-proposal.ts INPUT.proposal.txt OUTPUT.edits.json");
const proposal = parseImageProposal(readFileSync(input, "utf8"));
writeFileSync(output, JSON.stringify({ ...proposal, settings: { width: 1024, height: 1024, preset: "standard", seed: 42621, batchSize: 1 } }, null, 2) + "\n");
if (previewOutput) writeFileSync(previewOutput, JSON.stringify(imageCompositionPreview({ ...sampleImage, ...proposal }), null, 2) + "\n");
console.log(`Applied ${proposal.elements.length} bounded elements with Kestrel's UI parser: ${output}`);
