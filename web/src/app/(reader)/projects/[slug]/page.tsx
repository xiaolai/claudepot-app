import Link from "next/link";
import { notFound } from "next/navigation";
import { ExternalLink, Star } from "lucide-react";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import {
  getProjectBySlug,
  getProjectTags,
  getRelatedSubmissionsForProject,
} from "@/db/queries";
import { renderEditorialDoc, renderProjectReadme } from "@/lib/markdown";
import { relativeDays } from "@/lib/format";

export default async function ProjectPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const project = await getProjectBySlug(slug);
  if (!project) notFound();

  const [projectTags, related] = await Promise.all([
    getProjectTags(slug),
    getRelatedSubmissionsForProject(slug),
  ]);

  const editorialHtml = project.editorial_md
    ? renderEditorialDoc(project.editorial_md)
    : null;
  const readmeHtml = project.readme_md
    ? renderProjectReadme(project.readme_md, project.repo_url)
    : null;

  return (
    <div className="proto-page-narrow">
      <header className="proto-project-hero">
        <h1>{project.name}</h1>
        <p className="proto-project-hero-tagline">
          {project.tagline || <em>(no description)</em>}
        </p>
        <div className="proto-project-meta">
          {project.repo_url && (
            <a
              href={project.repo_url}
              target="_blank"
              rel="noopener noreferrer"
              className="proto-project-meta-link"
            >
              github <ExternalLink size={12} aria-hidden />
            </a>
          )}
          {project.site_url && (
            <a
              href={project.site_url}
              target="_blank"
              rel="noopener noreferrer"
              className="proto-project-meta-link"
            >
              site <ExternalLink size={12} aria-hidden />
            </a>
          )}
          {project.primary_language && <span>{project.primary_language}</span>}
          <span>
            <Star size={12} aria-hidden fill="currentColor" /> {project.stars}
          </span>
          {project.updated_at && <span>{relativeDays(project.updated_at)}</span>}
        </div>
        {projectTags.length > 0 && (
          <ul className="proto-project-tags" aria-label="Project tags">
            {projectTags.map((t) => (
              <li key={t.slug}>
                <Link href={`/c/${t.slug}`} className="proto-project-tag-chip">
                  {t.name}
                </Link>
              </li>
            ))}
          </ul>
        )}
      </header>

      {editorialHtml && (
        <section
          className="proto-project-editorial"
          aria-label="Editor's note"
          dangerouslySetInnerHTML={{ __html: editorialHtml }}
        />
      )}

      {readmeHtml ? (
        <section
          className="proto-project-readme"
          aria-label={`${project.name} README`}
          dangerouslySetInnerHTML={{ __html: readmeHtml }}
        />
      ) : (
        <section className="proto-section">
          <p className="proto-empty">
            No README captured yet. Run{" "}
            <code>pnpm projects:seed</code> to refresh.
          </p>
        </section>
      )}

      <section className="proto-section">
        <h2>On ClauDepot</h2>
        {related.length > 0 ? (
          <ol className="proto-feed">
            {related.map((s) => (
              <SubmissionRow key={s.id} submission={s} />
            ))}
          </ol>
        ) : projectTags.length === 0 ? (
          <p className="proto-empty">
            Bind tags to this project in{" "}
            <code>design/fixtures/project-tags.json</code> to populate this
            list.
          </p>
        ) : (
          <p className="proto-empty">
            No submissions on ClauDepot yet matching{" "}
            {projectTags.map((t, i) => (
              <span key={t.slug}>
                {i > 0 && ", "}
                <code>#{t.slug}</code>
              </span>
            ))}
            .
          </p>
        )}
      </section>
    </div>
  );
}
