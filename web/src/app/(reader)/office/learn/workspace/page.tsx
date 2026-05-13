import { FolderCog } from "lucide-react";
import { readTutorialMd } from "@/lib/editorial-spec";
import { renderEditorialDoc } from "@/lib/markdown";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";
import { MermaidEnhancer } from "@/components/prototype/MermaidEnhancer";

export const dynamic = "force-static";

export default function WorkspacePage() {
  const html = renderEditorialDoc(readTutorialMd("workspace"));
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="learn-workspace" />
      <div className="proto-page-aside-content">
        <header className="proto-section">
          <div className="office-eyebrow">
            <FolderCog size={14} aria-hidden /> learn · workspace
          </div>
        </header>
        <article
          className="office-doc"
          dangerouslySetInnerHTML={{ __html: html }}
        />
        <MermaidEnhancer />
      </div>
    </div>
  );
}
