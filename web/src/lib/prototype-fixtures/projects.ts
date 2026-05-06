/**
 * Project queries against the prototype fixtures.
 */

import { projects, publicVisible } from "./data";
import type { Project, Submission } from "./types";

export function getAllProjects(): Project[] {
  return projects;
}

export function getProjectBySlug(slug: string): Project | undefined {
  return projects.find((p) => p.slug === slug);
}

export function getRelatedSubmissionsForProject(
  slug: string,
  limit = 4,
): Submission[] {
  const project = getProjectBySlug(slug);
  if (!project) return [];
  const tokens = [project.slug, project.name.toLowerCase()];
  return publicVisible()
    .filter((s) =>
      tokens.some(
        (t) =>
          s.title.toLowerCase().includes(t) ||
          s.url?.toLowerCase().includes(t) ||
          false,
      ),
    )
    .slice(0, limit);
}
