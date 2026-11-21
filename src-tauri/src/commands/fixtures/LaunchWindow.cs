// Windows launch regression fixture. The manager's window probe shows the HWND.
using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;

sealed class LaunchWindow : Form {
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr hwnd);

    static readonly string Root = AppDomain.CurrentDomain.BaseDirectory;
    static readonly int Pid = Process.GetCurrentProcess().Id;
    static bool finishing;

    static void Record(string file) {
        File.AppendAllText(Path.Combine(Root, file), Pid + Environment.NewLine);
    }

    protected override void WndProc(ref Message message) {
        if (message.Msg == 0x10 && !finishing) Record("unexpected-close");
        base.WndProc(ref message);
    }

    [STAThread]
    static void Main(string[] args) {
        bool helper = Array.IndexOf(args, "--type=utility") >= 0;
        bool successor = Array.IndexOf(args, "--successor") >= 0;
        if (helper || successor) {
            Record(helper ? "helpers" : "successors");
            var awaiting = Stopwatch.StartNew();
            while (!File.Exists(Path.Combine(Root, "stop-" + Pid)) &&
                   !File.Exists(Path.Combine(Root, "stop-all")) && awaiting.ElapsedMilliseconds < 90000) {
                if (successor && File.Exists(Path.Combine(Root, "show-successor"))) break;
                System.Threading.Thread.Sleep(40);
            }
            if (!successor || !File.Exists(Path.Combine(Root, "show-successor"))) return;
        }
        foreach (string role in new string[] { "helper", "successor" }) {
            string flag = Path.Combine(Root, "spawn-" + role);
            if (!File.Exists(flag)) continue;
            File.Delete(flag);
            using (Process.Start(new ProcessStartInfo {
                FileName = Process.GetCurrentProcess().MainModule.FileName,
                Arguments = role == "helper" ? "--type=utility" : "--successor",
                UseShellExecute = false, CreateNoWindow = true
            })) {}
        }
        if (File.Exists(Path.Combine(Root, "observe-relaunch-home"))) {
            File.WriteAllText(Path.Combine(Root, "relaunch-home-" + Pid), Environment.GetEnvironmentVariable("CODEX_HOME"));
        }
        using (var form = new LaunchWindow())
        using (var timer = new Timer { Interval = 40 }) {
            form.Text = "Codex isolated launch regression";
            form.Size = new Size(640, 420);
            IntPtr hwnd = form.Handle;
            Record("pids");
            var lifetime = Stopwatch.StartNew();
            var observed = new Stopwatch();
            bool transient = File.Exists(Path.Combine(Root, "transient"));
            int stage = 0;
            timer.Tick += delegate {
                if (!observed.IsRunning && IsWindowVisible(hwnd)) observed.Start();
                if (transient && stage == 0 && observed.ElapsedMilliseconds >= 600) {
                    form.Size = new Size(160, 100);
                    Record("temporarily-small");
                    stage = 1;
                } else if (stage == 1 && observed.ElapsedMilliseconds >= 4900) {
                    form.Size = new Size(640, 420);
                    Record("restored");
                    stage = 2;
                }
                if (File.Exists(Path.Combine(Root, "stop-" + Pid)) ||
                    File.Exists(Path.Combine(Root, "stop-all")) || lifetime.ElapsedMilliseconds > 90000) {
                    finishing = true;
                    Application.ExitThread();
                }
            };
            timer.Start();
            Application.Run();
        }
    }
}
