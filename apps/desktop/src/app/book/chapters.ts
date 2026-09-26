import {
  Clapperboard,
  Download,
  Headphones,
  Image as ImageIcon,
  Library,
  MessageSquare,
  MonitorCog,
  Wrench,
  type LucideIcon,
} from "lucide-react";

/** The Kestrel book, in binding order. Each chapter is one retained workspace. */
export type AppView = "setup" | "control" | "research" | "studio" | "image" | "music" | "developer" | "system";

export type Chapter = {
  id: AppView;
  /** The index-tab label and the accessible name of its button. */
  label: string;
  numeral: string;
  /** Running head on the right page: what this chapter is for. */
  purpose: string;
  icon: LucideIcon;
};

export const CHAPTERS: readonly Chapter[] = [
  { id: "setup", label: "Setup", numeral: "I", purpose: "Install once, then work offline", icon: Download },
  { id: "control", label: "Control", numeral: "II", purpose: "Chat and visible computer work", icon: MessageSquare },
  { id: "research", label: "Research", numeral: "III", purpose: "Private library and reading room", icon: Library },
  { id: "studio", label: "Studio", numeral: "IV", purpose: "Story, scenes, edit and delivery", icon: Clapperboard },
  { id: "image", label: "Image", numeral: "V", purpose: "Composed images and takes", icon: ImageIcon },
  { id: "music", label: "Music", numeral: "VI", purpose: "Arrangement, lyrics and takes", icon: Headphones },
  { id: "developer", label: "Developer", numeral: "VII", purpose: "Offline diagnostics and repair", icon: Wrench },
  { id: "system", label: "System", numeral: "VIII", purpose: "One runtime policy for every model", icon: MonitorCog },
];

export function chapterIndex(view: AppView): number {
  return CHAPTERS.findIndex((chapter) => chapter.id === view);
}

export function chapterOf(view: AppView): Chapter {
  return CHAPTERS[chapterIndex(view)] ?? CHAPTERS[0];
}
