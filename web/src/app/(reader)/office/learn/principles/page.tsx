import { GraduationCap } from "lucide-react";
import { readTutorialMd } from "@/lib/editorial-spec";
import { renderEditorialDoc } from "@/lib/markdown";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";
import { MermaidEnhancer } from "@/components/prototype/MermaidEnhancer";

export const dynamic = "force-static";

export default function PrinciplesPage() {
  const html = renderEditorialDoc(readTutorialMd("principles"));
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="learn-principles" />
      <div className="proto-page-aside-content">
        <header className="proto-section">
          <div className="office-eyebrow">
            <GraduationCap size={14} aria-hidden /> learn · principles
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
