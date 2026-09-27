/**
 * A production asking Image Studio for a picture. It lives only in memory while the producer makes
 * the picture: Image Studio shows what it is for and hands the chosen take back, and the production
 * imports that take natively (by project and take ID) into its reference library.
 */
export type ImageHandoff = {
  /** What the picture is for, such as "First frame". */
  purpose: string;
  /** The production the picture returns to. */
  production: string;
  /** The producer's description, used to start a new image project. */
  direction: string;
  /** Imports the chosen take and puts it where the producer asked for it. */
  deliver: (imageProjectId: string, takeId: string) => Promise<void>;
};
