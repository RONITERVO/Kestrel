import { invoke, listen } from "./transport";
import type { MovieProject } from "../contracts/index";

// Never executed. tsc proves that the native boundary rejects these invalid call sites.
export function checkNativeBoundaryTypes() {
  const movie: Promise<MovieProject> = invoke("get_movie", { id: "movie" });
  void movie;
  void invoke("bootstrap");
  // @ts-expect-error Unknown commands cannot cross the boundary.
  void invoke("invented_command", {});
  // @ts-expect-error Rust requires id, and it must be text.
  void invoke("get_movie", { id: 123 });
  // @ts-expect-error Required arguments cannot be omitted.
  void invoke("get_movie");
  // @ts-expect-error A caller cannot invent a different result type.
  const wrong: Promise<string> = invoke("get_movie", { id: "movie" });
  void wrong;
  // @ts-expect-error Native event names are generated too.
  void listen("invented-event", () => undefined);
  void listen("movie-render-preview", (event) => {
    const step: number | undefined = event.payload.step;
    void step;
    // @ts-expect-error Preview payloads do not contain authoritative project state.
    void event.payload.edit;
  });
}
