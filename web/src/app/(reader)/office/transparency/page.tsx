import { Cpu } from "lucide-react";
import { readTransparencyMd } from "@/lib/editorial-spec";
import { renderEditorialDoc } from "@/lib/markdown";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";

export const dynamic = "force-static";

export default function TransparencyPage() {
  const html = renderEditorialDoc(readTransparencyMd());
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="transparency" />
      <div className="proto-page-aside-content">
        <header className="proto-section">
          <div className="office-eyebrow">
            <Cpu size={14} aria-hidden /> transparency policy
          </div>
        </header>
        <article
          className="office-doc"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </div>
    </div>
  );
}
