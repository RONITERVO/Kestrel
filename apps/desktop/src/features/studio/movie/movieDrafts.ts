import type { MovieReferenceAsset, ProducerReferenceRequest } from "../../../contracts/index";

/** Unsubmitted reference form; the native attach command validates and saves it. */
export interface PendingMovieReference extends MovieReferenceAsset, ProducerReferenceRequest {}
