import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Help",
  description: "Help articles for ClauDepot users and Claudepot operators.",
};

const ARTICLES: Array<{ href: string; title: string; dek: string }> = [
  {
    href: "/help/network",
    title: "Network requirements & troubleshooting",
    dek: "What endpoints Claudepot needs, how to diagnose unreachability, and how to use a third-party LLM when Anthropic's API isn't reachable.",
  },
];

export default function HelpIndex() {
  return (
    <div className="proto-page-narrow">
      <h1>Help</h1>
      <p className="proto-dek">
        Practical articles for the parts of Claudepot that touch your
        environment — network, account state, troubleshooting.
      </p>
      <ul className="proto-help-index">
        {ARTICLES.map((a) => (
          <li key={a.href}>
            <h2>
              <Link href={a.href}>{a.title}</Link>
            </h2>
            <p>{a.dek}</p>
          </li>
        ))}
      </ul>
    </div>
  );
}
