import type { ComponentProps } from "react";
import { ReplyPages } from "../../shared/book/ReplyPages";
import { useInferenceTelemetryReporter } from "./InferenceTelemetry";

/** A live Studio proposal: the shared reply pages plus the banner's local speed telemetry. */
export function ModelReplyPages({
  modelName,
  inferenceActive,
  ...reply
}: ComponentProps<typeof ReplyPages> & { modelName?: string; inferenceActive?: boolean }) {
  useInferenceTelemetryReporter({ active: inferenceActive ?? reply.live, text: reply.reasoning + reply.text, modelName });
  return <ReplyPages {...reply} />;
}
