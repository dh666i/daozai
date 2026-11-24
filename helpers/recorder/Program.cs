using System;
using System.Collections.Concurrent;
using System.IO;
using System.Text;
using System.Threading;
using ScreenRecorderLib;

internal static class Program
{
    private static int Main(string[] args)
    {
        Console.OutputEncoding = new UTF8Encoding(false);
        try
        {
            return Run(args, null);
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(error.Message);
            return 1;
        }
    }

    internal static int Run(string[] args, RecordingSourceBase source)
    {
        string output = null;
        bool systemAudio = false;
        bool microphone = false;
        for (int i = 0; i < args.Length; ++i)
        {
            switch (args[i])
            {
                case "--output":
                    if (output != null || ++i == args.Length)
                        throw new ArgumentException("Expected one output path.");
                    output = Path.GetFullPath(args[i]);
                    break;
                case "--system-audio": systemAudio = true; break;
                case "--microphone": microphone = true; break;
                default: throw new ArgumentException("Unknown argument: " + args[i]);
            }
        }
        if (output == null || !String.Equals(Path.GetExtension(output), ".mp4", StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("Expected --output <file.mp4>.");
        if (File.Exists(output))
            throw new IOException("The output file already exists.");
        string directory = Path.GetDirectoryName(output);
        Directory.CreateDirectory(directory);
        string partial = Path.Combine(directory, ".daozai-" + Guid.NewGuid().ToString("N") + ".mp4");

        RecorderOptions options = RecorderOptions.DefaultMainMonitor;
        if (source != null)
        {
            options.SourceOptions.RecordingSources.Clear();
            options.SourceOptions.RecordingSources.Add(source);
        }
        options.VideoEncoderOptions.Framerate = 30;
        options.VideoEncoderOptions.IsFixedFramerate = true;
        options.AudioOptions.IsAudioEnabled = systemAudio || microphone;
        options.AudioOptions.AudioSources.Clear();
        if (systemAudio)
            options.AudioOptions.AudioSources.AddRange(AudioOptions.DefaultAudioPlaybackDevice.AudioSources);
        if (microphone)
            options.AudioOptions.AudioSources.AddRange(AudioOptions.DefaultAudioCaptureDevice.AudioSources);

        try
        {
            using (Recorder recorder = Recorder.CreateRecorder(options))
            {
                Capture(recorder, partial);
            }
            if (!File.Exists(partial) || new FileInfo(partial).Length == 0)
                throw new IOException("The encoder returned an empty recording.");
            File.Move(partial, output);
        }
        finally
        {
            if (File.Exists(partial)) File.Delete(partial);
        }
        Console.WriteLine("DONE");
        return 0;
    }

    private static void Capture(Recorder recorder, string partial)
    {
        var messages = new BlockingCollection<string>();
        recorder.OnRecordingComplete += (sender, result) => messages.Add("complete");
        recorder.OnRecordingFailed += (sender, result) => messages.Add("error:" + result.Error);
        recorder.OnStatusChanged += (sender, result) => messages.Add("state:" + result.Status);

        // Only this thread changes the recorder. Driver callbacks and stdin queue their events.
        var input = new Thread(delegate()
        {
            try
            {
                string line;
                while ((line = Console.ReadLine()) != null)
                {
                    messages.Add(line == "pause" || line == "resume" || line == "stop" ? line : "invalid");
                    if (line == "stop") return;
                }
            }
            catch (IOException) { }
            messages.Add("stop");
        });
        input.IsBackground = true;
        input.Start();
        recorder.Record(partial);

        bool ready = false, stopping = false, stopRequested = false;
        DateTime deadline = DateTime.UtcNow.AddSeconds(25);
        for (;;)
        {
            string message;
            int timeout = ready && !stopping ? Timeout.Infinite :
                (int)Math.Max(0, (deadline - DateTime.UtcNow).TotalMilliseconds);
            if (!messages.TryTake(out message, timeout))
                throw new TimeoutException(stopping ? "Recording finalization timed out." : "Recording did not start.");
            if (message.StartsWith("error:", StringComparison.Ordinal))
                throw new IOException(message.Substring(6));
            if (message == "complete") return;
            if (message == "state:Recording")
            {
                if (!stopping) Console.WriteLine(ready ? "RESUMED" : "READY");
                ready = true;
            }
            else if (message == "state:Paused" && !stopping)
                Console.WriteLine("PAUSED");
            else if (message == "stop")
                stopRequested = true;
            else if (message == "pause" && ready && !stopping && recorder.Status == RecorderStatus.Recording)
                recorder.Pause();
            else if (message == "resume" && ready && !stopping && recorder.Status == RecorderStatus.Paused)
                recorder.Resume();

            if (stopRequested && ready && !stopping)
            {
                stopping = true;
                deadline = DateTime.UtcNow.AddSeconds(30);
                recorder.Stop();
            }
        }
    }
}
