import { BehaviorSubject } from 'rxjs';

export type Report = {
  title: string;
  level: number;
  content: string[];
  date: string;
};
export type Table = { headers: string[]; rows: string[][] };
export type Status = {
  label: string;
  tone: 'bad' | 'warn' | 'good' | 'neutral';
};
export type RunHistoryItem = {
  report: Report;
  status: Status;
  tables: Table[];
  summary: string;
  detailReports: Report[];
};
export type BenchmarkReportState = {
  loading: boolean;
  error: string;
  reports: Report[];
  latestRun?: Report;
  status: Status;
  currentTables: Table[];
  runHistory: RunHistoryItem[];
  baselineDate: string;
  latestSummary: string;
};

export function parseReports(markdown: string): Report[] {
  const reports: Report[] = [];
  let current: Report | undefined;
  for (const line of markdown.split(/\r?\n/)) {
    const heading = /^(#{2,3})\s+(.+)$/.exec(line);
    if (heading) {
      current = {
        title: heading[2],
        level: heading[1].length,
        content: [],
        date: heading[2].match(/20\d{2}-\d{2}-\d{2}/)?.[0] ?? '',
      };
      reports.push(current);
    } else if (current) {
      current.content.push(line);
      if (!current.date)
        current.date = line.match(/20\d{2}-\d{2}-\d{2}/)?.[0] ?? '';
    }
  }
  return reports;
}

export function readTables(lines: string[]): Table[] {
  const tables: Table[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    if (!lines[index].trim().startsWith('|')) continue;
    const block: string[] = [];
    while (index < lines.length && lines[index].trim().startsWith('|'))
      block.push(lines[index++]);
    index -= 1;
    const cells = (line: string) =>
      line
        .trim()
        .replace(/^\|/, '')
        .replace(/\|$/, '')
        .split('|')
        .map((cell) => cell.trim());
    const headers = cells(block[0]);
    const rows = block
      .slice(1)
      .filter((line) => !/^\|?\s*:?-{3,}/.test(line.trim()))
      .map(cells);
    if (rows.length) tables.push({ headers, rows });
  }
  return tables;
}

export function plainText(lines: string[]) {
  return lines
    .filter(
      (line) =>
        line.trim() &&
        !line.trim().startsWith('|') &&
        !line.trim().startsWith('#'),
    )
    .map((line) =>
      line
        .replace(/^\s*[-*]\s+/, '')
        .replace(/\*\*/g, '')
        .replace(/`/g, '')
        .trim(),
    );
}

export function inferStatus(text: string): Status {
  if (/failed|failure|did not compile|error:/i.test(text))
    return { label: 'Failure recorded', tone: 'bad' };
  if (/skipped|not run|unset/i.test(text))
    return { label: 'Some targets skipped', tone: 'warn' };
  if (
    /passed|completed successfully|all benchmark targets completed/i.test(text)
  )
    return { label: 'Completed', tone: 'good' };
  return { label: 'Recorded', tone: 'neutral' };
}

export function formatDate(value: string) {
  if (!value) return 'Date not recorded';
  return new Intl.DateTimeFormat('en-GB', {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    timeZone: 'UTC',
  }).format(new Date(`${value}T00:00:00Z`));
}

function deriveState(
  markdown: string,
  loading: boolean,
  error: string,
): BenchmarkReportState {
  const reports = parseReports(markdown);
  const isRun = (report: Report) =>
    report.title.startsWith('Follow-up run') ||
    report.title.startsWith('Repeat run') ||
    report.title.startsWith('Final follow-up run');
  const latestRun = reports.filter(isRun)[0];
  const runHistory = reports
    .filter((report) => report.level === 2 && isRun(report))
    .map((report) => {
      const index = reports.indexOf(report);
      const nextIndex = reports.findIndex(
        (item, next) => next > index && item.level === 2,
      );
      const detailReports = reports.slice(
        index + 1,
        nextIndex < 0 ? undefined : nextIndex,
      );
      const text = [
        report.title,
        ...report.content,
        ...detailReports.flatMap((item) => [item.title, ...item.content]),
      ].join('\n');
      const tables = [
        ...readTables(report.content),
        ...detailReports.flatMap((item) => readTables(item.content)),
      ];
      const summary =
        plainText(report.content).slice(0, 2).join(' ') ||
        plainText(detailReports.flatMap((item) => item.content))
          .slice(0, 1)
          .join(' ');
      return {
        report,
        status: inferStatus(text),
        tables,
        summary,
        detailReports,
      };
    });
  return {
    loading,
    error,
    reports,
    latestRun,
    status: inferStatus(
      latestRun ? [latestRun.title, ...latestRun.content].join('\n') : '',
    ),
    currentTables: latestRun ? readTables(latestRun.content) : [],
    runHistory,
    baselineDate:
      reports.find((item) => item.title === 'Results')?.date ?? '2026-11-02',
    latestSummary: plainText(latestRun?.content ?? [])
      .slice(0, 2)
      .join(' '),
  };
}

export class BenchmarkReportModel {
  readonly state$ = new BehaviorSubject<BenchmarkReportState>(
    deriveState('', true, ''),
  );
  private started = false;
  private request?: AbortController;

  start() {
    if (this.started) return;
    this.started = true;
    this.request = new AbortController();
    fetch(`${import.meta.env.BASE_URL}benchmark-baseline.md`, {
      signal: this.request.signal,
    })
      .then((response) => {
        if (!response.ok)
          throw new Error(
            `Could not load benchmark README (${response.status}).`,
          );
        return response.text();
      })
      .then((text) => this.state$.next(deriveState(text, false, '')))
      .catch((reason: unknown) => {
        if (reason instanceof DOMException && reason.name === 'AbortError')
          return;
        this.state$.next(
          deriveState(
            '',
            false,
            reason instanceof Error
              ? reason.message
              : 'Could not load the benchmark README.',
          ),
        );
      });
  }

  stop() {
    this.request?.abort();
    this.request = undefined;
    this.started = false;
  }
}
