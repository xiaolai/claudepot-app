import Link from "next/link";
import { ExternalLink, Star } from "lucide-react";
import { getAllProjects } from "@/db/queries";
import { randomCardTint } from "@/lib/card-tint";
import { relativeDays } from "@/lib/format";

const PIN_FIRST = "claudepot-app";

export default async function ProjectsHub() {
  const projects = await getAllProjects();
  // Pin claudepot-app first; sort the rest by stars desc.
  const pinned = projects.filter((p) => p.slug === PIN_FIRST);
  const rest = projects
    .filter((p) => p.slug !== PIN_FIRST)
    .sort((a, b) => b.stars - a.stars);
  const sorted = [...pinned, ...rest];

  return (
    <div className="proto-page">
      <h1>Projects</h1>
      <p className="proto-dek">
        Open-source projects by{" "}
        <a href="https://lixiaolai.com" target="_blank" rel="noopener noreferrer">
          @xiaolai
        </a>{" "}
        across the AI tools field.
      </p>
      <div className="proto-projects-grid">
        {sorted.map((p) => {
          const tint = randomCardTint();
          return (
            <article
              key={p.slug}
              className="proto-project-card proto-project-card-tinted"
              style={{ ["--card-tint" as string]: tint }}
            >
              <Link
                href={`/projects/${p.slug}`}
                className="proto-project-card-body"
              >
                <h2 className="proto-project-card-name">{p.name}</h2>
                <p className="proto-project-card-tagline">
                  {p.tagline || <em>(no description)</em>}
                </p>
              </Link>
              <div className="proto-project-card-meta">
                {p.primary_language && <span>{p.primary_language}</span>}
                <span>
                  <Star size={12} aria-hidden fill="currentColor" /> {p.stars}
                </span>
                {p.updated_at && <span>{relativeDays(p.updated_at)}</span>}
                {p.repo_url && (
                  <a
                    href={p.repo_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="proto-project-card-repo"
                    aria-label={`${p.name} on GitHub`}
                  >
                    github <ExternalLink size={12} aria-hidden />
                  </a>
                )}
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}
