import { Link } from "@tanstack/react-router";
import { ArrowRight, Plus } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { Chip, ChipList } from "@/components/nodes/chip";
import { Figure, FigureGrid } from "@/components/nodes/figure";
import { HueDot } from "@/components/nodes/hue-dot";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { StepFlow, StepNode } from "@/components/nodes/step-node";
import { PageHeader } from "@/components/shell/page-header";
import { ThemeScope } from "@/components/theme/theme-scope";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Slider } from "@/components/ui/slider";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { hueFor } from "@/lib/hue";
import { strings } from "@/lib/strings";
import { PALETTES, type Mode } from "@/theme/preferences";

const MODES: readonly Mode[] = ["light", "dark"];

/** Fixture entities: the kinds of things that get a hashed hue. */
const SAMPLE_ENTITIES = [
  { kind: "user", name: "Ada", id: "0193a5b2-1c4d-7e8f-a1b2-c3d4e5f60718" },
  { kind: "agent", name: "Triage bot", id: "018f3c1e-7b2a-7c3d-9e4f-5a6b7c8d9e0f" },
  { kind: "teamspace", name: "Design", id: "f47ac10b-58cc-4372-a567-0e02b2c3d479" },
  { kind: "database", name: "Roadmap", id: "9c5b94b1-35ad-49bb-b118-8e8fc24abf80" },
] as const;

/** The parts a theme has to get right, in one block small enough to repeat. */
export function StyleguidePage() {
  const copy = strings.styleguide;
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />
      <div className="grid gap-12">
        <Section title={copy.type}>
          <TypeSamples />
        </Section>
        <Section title={copy.buttons}>
          <ButtonSamples />
        </Section>
        <Section title={copy.nodes}>
          <div className="grid gap-8">
            <Subsection title={copy.keyValue}>
              <KeyValueSample />
            </Subsection>
            <Subsection title={copy.chips}>
              <ChipSample />
            </Subsection>
            <Subsection title={copy.steps}>
              <StepSample />
            </Subsection>
            <Subsection title={copy.figures}>
              <FigureSample />
            </Subsection>
          </div>
        </Section>
        <Section title={copy.controls}>
          <ControlSamples />
        </Section>
        <Section title={copy.hues} hint={copy.huesHint}>
          <HueSamples />
        </Section>
        <Section title={copy.palettes} hint={copy.palettesHint}>
          <PaletteMatrix />
        </Section>
      </div>
    </>
  );
}

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="grid gap-5 border-t border-border pt-6">
      <div className="grid gap-1">
        <h2 id={headingId} className="text-xl font-semibold">
          {title}
        </h2>
        {hint !== undefined && <p className="text-sm text-muted-foreground">{hint}</p>}
      </div>
      {children}
    </section>
  );
}

function Subsection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="grid gap-3">
      <h3 className="font-mono text-xs tracking-wider text-muted-foreground">{title}</h3>
      {children}
    </div>
  );
}

function TypeSamples() {
  return (
    <div className="grid gap-3">
      <p className="text-3xl font-semibold">Pages, databases and boards</p>
      <p className="text-xl font-semibold">A heading one level down</p>
      <p className="max-w-[62ch]">
        Body text in the interface font. It sits on the dot grid in the palette&apos;s ink, with
        rules in the palette&apos;s line colour.
      </p>
      <p className="max-w-[62ch] text-muted-foreground">
        Muted text for hints and secondary detail, still above 4.5:1 on every ground.
      </p>
      <p className="max-w-[62ch] font-serif text-lg">Serif, for pages set in the serif face.</p>
      <p className="font-mono text-sm">Mono: iA Writer Mono, 0123456789 {"{ } => !="}</p>
      <p>
        <Link to="/" className="text-primary underline underline-offset-4">
          An inline link in the accent
        </Link>
      </p>
    </div>
  );
}

function ButtonSamples() {
  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button>Primary</Button>
        <Button variant="pill">Pill</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="link">Link</Button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm">Small</Button>
        <Button size="lg">
          Large
          <ArrowRight aria-hidden />
        </Button>
        <Button size="icon" aria-label="Add">
          <Plus aria-hidden />
        </Button>
        <Button size="icon" variant="outline" aria-label="Add">
          <Plus aria-hidden />
        </Button>
        <Button disabled>Disabled</Button>
      </div>
    </div>
  );
}

function KeyValueSample() {
  return (
    <KeyValueList className="max-w-xl">
      <KeyValueRow label="Workspace">Hackathon crew</KeyValueRow>
      <KeyValueRow label="Members">4 people, 2 agents</KeyValueRow>
      <KeyValueRow label="Created">
        <span className="font-mono text-sm tabular-nums">2026-11-21</span>
      </KeyValueRow>
    </KeyValueList>
  );
}

function ChipSample() {
  return (
    <ChipList>
      <Chip tone="strong">Selected</Chip>
      <Chip>Draft</Chip>
      <Chip>In review</Chip>
      {SAMPLE_ENTITIES.map((entity) => (
        <Chip key={entity.id} hue={hueFor(entity.id)}>
          {entity.name}
        </Chip>
      ))}
    </ChipList>
  );
}

function StepSample() {
  return (
    <StepFlow>
      <StepNode title="Browser" detail="Edits a block" timing="0 ms" />
      <StepNode title="Server" detail="Merges the change" timing="4 ms" active />
      <StepNode title="Teammates" detail="See it live" timing="9 ms" />
    </StepFlow>
  );
}

function FigureSample() {
  return (
    <FigureGrid>
      <Figure value="12" label="Pages" caption="Across 3 teamspaces" />
      <Figure value="48" label="Cards" caption="On the roadmap board" />
      <Figure value="2" label="Agents" caption="Connected over MCP" />
      <Figure value="0" label="Conflicts" caption="Merged, not overwritten" />
    </FigureGrid>
  );
}

function ControlSamples() {
  const [density, setDensity] = useState("comfortable");
  const [volume, setVolume] = useState(40);
  const densityId = useId();
  return (
    <div className="grid max-w-xl gap-5">
      <div className="grid gap-2">
        <span id={densityId} className="text-sm">
          Density
        </span>
        <ToggleGroup
          type="single"
          aria-labelledby={densityId}
          value={density}
          onValueChange={(value) => {
            if (value) {
              setDensity(value);
            }
          }}
        >
          <ToggleGroupItem value="compact">Compact</ToggleGroupItem>
          <ToggleGroupItem value="comfortable">Comfortable</ToggleGroupItem>
          <ToggleGroupItem value="spacious">Spacious</ToggleGroupItem>
        </ToggleGroup>
      </div>
      <Separator />
      <div className="grid gap-2">
        <span className="text-sm">Volume: {volume}%</span>
        <Slider
          label="Volume"
          valueText={`${volume}%`}
          min={0}
          max={100}
          value={[volume]}
          onValueChange={([next]) => {
            if (next !== undefined) {
              setVolume(next);
            }
          }}
        />
      </div>
    </div>
  );
}

function HueSamples() {
  return (
    <KeyValueList className="max-w-xl">
      {SAMPLE_ENTITIES.map((entity) => (
        <KeyValueRow
          key={entity.id}
          label={
            <span className="flex items-center gap-2">
              <HueDot hue={hueFor(entity.id)} />
              {entity.name}
              <span className="text-xs">({entity.kind})</span>
            </span>
          }
        >
          <span className="font-mono text-sm tabular-nums">
            {entity.id.slice(-8)} → {hueFor(entity.id)}°
          </span>
        </KeyValueRow>
      ))}
    </KeyValueList>
  );
}

function PaletteMatrix() {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {PALETTES.flatMap((palette) =>
        MODES.map((mode) => (
          <ThemeScope
            key={`${palette}-${mode}`}
            palette={palette}
            mode={mode}
            className="grid gap-4 rounded-lg border border-border dot-grid p-4"
          >
            <p className="flex items-center justify-between gap-2 font-mono text-xs tracking-wider">
              <span>{strings.appearance.palettes[palette]}</span>
              <span className="text-muted-foreground">{strings.appearance.modes[mode]}</span>
            </p>
            <div className="flex flex-wrap gap-2">
              <Button size="sm">Primary</Button>
              <Button size="sm" variant="pill">
                Pill
              </Button>
              <Button size="sm" variant="outline">
                Outline
              </Button>
            </div>
            <ChipList>
              <Chip tone="strong">Selected</Chip>
              <Chip>Draft</Chip>
              <Chip hue={hueFor(SAMPLE_ENTITIES[0].id)}>{SAMPLE_ENTITIES[0].name}</Chip>
            </ChipList>
            <KeyValueList className="text-sm">
              <KeyValueRow label="Ground">Ink on the dot grid</KeyValueRow>
              <KeyValueRow label="Muted">
                <span className="text-muted-foreground">Hints and detail</span>
              </KeyValueRow>
            </KeyValueList>
            <StepFlow>
              <StepNode title="Draft" />
              <StepNode title="Review" active />
            </StepFlow>
          </ThemeScope>
        )),
      )}
    </div>
  );
}
