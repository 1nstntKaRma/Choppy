using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using Choppy.Engine;
using Choppy.UI;

namespace Choppy
{
    /// <summary>
    /// Runs jobs one at a time in the order they came in (two copies fighting over the same disk or line are
    /// slower than one after the other). Every queued job is on disk until it ends, so a crash can be resumed.
    /// </summary>
    internal sealed class JobManager
    {
        readonly object gate = new object();
        readonly List<JobSpec> waiting = new List<JobSpec>();
        readonly List<CopyJob> finished = new List<CopyJob>();
        public CopyJob Current { get; private set; }
        public CopyJob LastFinished { get; private set; }
        public event Action Changed;

        /// <summary>Conflict questions go to the UI thread (set by App).</summary>
        public Func<CopyJob, ConflictInfo, ConflictAnswer> Ask;
        public Func<CopyJob, long, long, bool> AskLowSpace;
        public Func<CopyJob, bool> AskElevate;
        public Action<CopyJob> JobStarted;
        public Action<CopyJob> AllDone;

        public int WaitingCount { get { lock (gate) return waiting.Count; } }
        public bool HasWaiting { get { return WaitingCount > 0; } }
        public bool Busy { get { lock (gate) return (Current != null && !Current.IsFinished) || waiting.Count > 0; } }
        public List<JobSpec> WaitingSnapshot() { lock (gate) return waiting.ToList(); }
        public List<CopyJob> FinishedSnapshot() { lock (gate) return finished.AsEnumerable().Reverse().Take(8).ToList(); }

        public void Enqueue(JobSpec spec)
        {
            var s = Settings.Current;
            if (spec.SpeedLimitMbps <= 0) spec.SpeedLimitMbps = s.SpeedLimitMbps;
            Journal.SaveSoon(spec);
            bool start;
            lock (gate)
            {
                waiting.Add(spec);
                start = Current == null || Current.IsFinished;
            }
            if (start) StartNext(); else Raise();
        }

        public void Remove(JobSpec spec)
        {
            lock (gate) waiting.Remove(spec);
            Journal.Remove(spec);
            Raise();
        }

        public void CancelCurrent()
        {
            var c = Current;
            if (c != null && !c.IsFinished) c.Cancel();
        }

        public void RetryFailed(CopyJob job)
        {
            var spec = new JobSpec
            {
                Op = job.Spec.Op,
                Dest = job.Spec.Dest,
                Verify = job.Spec.Verify,
                Policy = job.Spec.Policy == ConflictPolicy.Ask ? ConflictPolicy.Ask : job.Spec.Policy,
                Origin = "Retry",
                ExplicitItems = job.FailedItems.Select(f => new FileItem { Src = f.Src, Rel = f.Rel, Size = f.Size }).ToList(),
                Sources = job.Spec.Sources.ToList()
            };
            Enqueue(spec);
        }

        /// <summary>Speed limit changes apply to the running copy at once.</summary>
        public void ApplyLiveSettings()
        {
            var c = Current;
            if (c != null && !c.IsFinished) c.SetSpeedLimit(Settings.Current.SpeedLimitMbps);
        }

        void StartNext()
        {
            JobSpec spec;
            lock (gate)
            {
                if (Current != null && !Current.IsFinished) return;
                if (waiting.Count == 0) { Current = null; spec = null; }
                else { spec = waiting[0]; waiting.RemoveAt(0); }
            }
            if (spec == null) { Raise(); var h = AllDone; if (h != null) h(LastFinished); return; }

            var job = new CopyJob(spec);
            job.AskConflict = info => Ask != null ? Ask(job, info) : new ConflictAnswer { Choice = ConflictPolicy.Skip };
            job.AskLowSpace = (need, free) => AskLowSpace == null || AskLowSpace(job, need, free);
            job.AskElevate = () => AskElevate != null && AskElevate(job);
            job.PhaseChanged += j => Raise();
            job.Finished += OnFinished;
            lock (gate) Current = job;
            var st = JobStarted;
            if (st != null) st(job);
            job.Start();
            Raise();
        }

        void OnFinished(CopyJob job)
        {
            // canceled by the user or done = nothing to resume. A hard failure (destination gone) keeps its record.
            if (job.Phase != JobPhase.Failed) Journal.Remove(job.Spec);
            lock (gate)
            {
                finished.Add(job);
                LastFinished = job;
                if (Current == job) Current = null;
            }
            var app = Application.Current;
            if (app != null) app.Dispatcher.BeginInvoke(new Action(StartNext));
            else StartNext();
        }

        void Raise()
        {
            var h = Changed;
            if (h == null) return;
            var app = Application.Current;
            if (app != null && !app.Dispatcher.CheckAccess()) app.Dispatcher.BeginInvoke(new Action(() => { var hh = Changed; if (hh != null) hh(); }));
            else h();
        }
    }
}
