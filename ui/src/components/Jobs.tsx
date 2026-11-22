import { CircleCheck, Hourglass, Play, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, onJobsChanged, type Job } from '../api';
import type { Run } from '../useTasks';

/** What tasks are waiting on, kept fresh as jobs start and finish. */
export function useJobs(): Job[] {
  const [jobs, setJobs] = useState<Job[]>([]);
  useEffect(() => {
    const load = () => void api.jobs().then(setJobs, () => {});
    load();
    return onJobsChanged(load);
  }, []);
  return jobs;
}

/** "<1m", "4m", "1h 5m". */
function since(iso: string, now: number): string {
  const minutes = Math.floor((now - new Date(iso).getTime()) / 60_000);
  if (minutes < 1) return '<1m';
  return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** Finished jobs ask to go back to their task; running ones show what's still cooking. */
export function JobNotes({ jobs, now, run }: { jobs: Job[]; now: number; run: Run }) {
  const done = jobs.filter((j) => j.finished_at);
  // An AI reply is only worth a line once it's ready; while it's written, the chat itself shows it.
  const waiting = jobs.filter((j) => !j.finished_at && j.what.kind !== 'tab');

  return (
    <>
      {done.map((job) => (
        <div key={`job-${job.id}`} className="suggestion job-done" role="status">
          <CircleCheck size={14} className="away-icon" aria-hidden />
          <div className="suggestion-text">
            <span className="suggestion-title" title={job.label}>
              {job.label} {job.what.kind === 'tab' ? 'replied' : 'is done'}
            </span>
            <span className="suggestion-reason" title={job.task_title}>
              {job.task_title ? `Back to “${job.task_title}”?` : 'Back to it?'}
            </span>
          </div>
          <button
            className="note-action"
            onClick={() => void run(() => api.backToJob(job.id))}
            aria-label={job.task_title ? `Back to "${job.task_title}"` : 'Back to it'}
            title={job.what.kind === 'tab' ? 'Go to the reply' : `Start “${job.task_title}” again`}
          >
            <Play size={14} />
          </button>
          <button
            className="icon-btn"
            onClick={() => void run(() => api.forgetJob(job.id))}
            aria-label="Not now"
            title="Not now"
          >
            <X size={14} />
          </button>
        </div>
      ))}

      {waiting.length > 0 && (
        <ul className="job-list" aria-label="Waiting on">
          {waiting.map((job) => (
            <li key={job.id} className="job-row">
              <Hourglass size={13} className="job-icon" aria-hidden />
              <span className="job-text" title={`${job.label} · for “${job.task_title}”`}>
                <span className="job-label">{job.label}</span>
                <span className="job-task">for {job.task_title}</span>
              </span>
              <span className="meta" title="Waiting for">
                {since(job.started_at, now)}
              </span>
              <button
                className="icon-btn"
                onClick={() => void run(() => api.forgetJob(job.id))}
                aria-label={`Stop waiting on ${job.label}`}
                title="Stop waiting"
              >
                <X size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
