import { Compass } from "lucide-react";
import { readTutorialMd } from "@/lib/editorial-spec";
import { renderEditorialDoc } from "@/lib/markdown";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";
import { MermaidEnhancer } from "@/components/prototype/MermaidEnhancer";

export const dynamic = "force-static";

export default function FormatsPage() {
  const html = renderEditorialDoc(readTutorialMd("formats"));
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="learn-formats" />
      <div className="proto-page-aside-content">
        <header className="proto-section">
          <div className="office-eyebrow">
            <Compass size={14} aria-hidden /> learn · formats
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
