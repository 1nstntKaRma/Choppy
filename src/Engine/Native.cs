using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Engine
{
    /// <summary>Win32 calls used by the engine. Every path passed here is already in \\?\ long form.</summary>
    internal static class Native
    {
        public const int ERROR_SUCCESS = 0;
        public const int ERROR_FILE_NOT_FOUND = 2;
        public const int ERROR_PATH_NOT_FOUND = 3;
        public const int ERROR_ACCESS_DENIED = 5;
        public const int ERROR_INVALID_HANDLE = 6;
        public const int ERROR_NOT_READY = 21;
        public const int ERROR_SHARING_VIOLATION = 32;
        public const int ERROR_LOCK_VIOLATION = 33;
        public const int ERROR_HANDLE_EOF = 38;
        public const int ERROR_BAD_NETPATH = 53;
        public const int ERROR_NETWORK_BUSY = 54;
        public const int ERROR_DEV_NOT_EXIST = 55;
        public const int ERROR_UNEXP_NET_ERR = 59;
        public const int ERROR_NETNAME_DELETED = 64;
        public const int ERROR_NETWORK_ACCESS_DENIED = 65;
        public const int ERROR_BAD_NET_NAME = 67;
        public const int ERROR_FILE_EXISTS = 80;
        public const int ERROR_DISK_FULL = 112;
        public const int ERROR_SEM_TIMEOUT = 121;
        public const int ERROR_INVALID_NAME = 123;
        public const int ERROR_DIR_NOT_EMPTY = 145;
        public const int ERROR_ALREADY_EXISTS = 183;
        public const int ERROR_FILENAME_EXCED_RANGE = 206;
        public const int ERROR_NO_MORE_FILES = 18;
        public const int ERROR_REQUEST_ABORTED = 1235;
        public const int ERROR_CONNECTION_ABORTED = 1236;
        public const int ERROR_NETWORK_UNREACHABLE = 1231;
        public const int ERROR_HOST_UNREACHABLE = 1232;
        public const int ERROR_CONNECTION_UNAVAIL = 1201;
        public const int ERROR_NOT_CONNECTED = 2250;
        public const int ERROR_HANDLE_DISK_FULL = 39;
        public const int ERROR_CRC = 23;
        public const int ERROR_IO_DEVICE = 1117;

        public const uint COPY_FILE_FAIL_IF_EXISTS = 0x00000001;
        public const uint COPY_FILE_ALLOW_DECRYPTED_DESTINATION = 0x00000008;
        public const uint COPY_FILE_NO_BUFFERING = 0x00001000;

        public const uint MOVEFILE_REPLACE_EXISTING = 0x1;
        public const uint MOVEFILE_COPY_ALLOWED = 0x2;
        public const uint MOVEFILE_WRITE_THROUGH = 0x8;

        public const uint PROGRESS_CONTINUE = 0;
        public const uint PROGRESS_CANCEL = 1;

        public const uint GENERIC_READ = 0x80000000;
        public const uint FILE_READ_ATTRIBUTES = 0x0080;
        public const uint FILE_WRITE_ATTRIBUTES = 0x0100;
        public const uint FILE_SHARE_READ = 1, FILE_SHARE_WRITE = 2, FILE_SHARE_DELETE = 4;
        public const uint OPEN_EXISTING = 3;
        public const uint FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
        public const uint FILE_FLAG_SEQUENTIAL_SCAN = 0x08000000;
        public const uint FILE_FLAG_NO_BUFFERING = 0x20000000;
        public const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;

        public const uint DRIVE_REMOVABLE = 2, DRIVE_FIXED = 3, DRIVE_REMOTE = 4, DRIVE_CDROM = 5, DRIVE_RAMDISK = 6;

        public const int INVALID_FILE_ATTRIBUTES = -1;
        public const uint FILE_ATTRIBUTE_READONLY = 0x1;
        public const uint FILE_ATTRIBUTE_DIRECTORY = 0x10;
        public const uint FILE_ATTRIBUTE_NORMAL = 0x80;
        public const uint FILE_ATTRIBUTE_REPARSE_POINT = 0x400;

        public delegate uint CopyProgressRoutine(long totalFileSize, long totalBytesTransferred, long streamSize,
            long streamBytesTransferred, uint streamNumber, uint callbackReason, IntPtr sourceFile, IntPtr destinationFile, IntPtr data);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool CopyFileExW(string existingFileName, string newFileName, CopyProgressRoutine progressRoutine,
            IntPtr data, ref int cancel, uint copyFlags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool MoveFileExW(string existingFileName, string newFileName, uint flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool DeleteFileW(string fileName);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool RemoveDirectoryW(string path);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool CreateDirectoryW(string path, IntPtr securityAttributes);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern int GetFileAttributesW(string fileName);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFileAttributesW(string fileName, uint attributes);

        // FILETIME is two DWORDs: 4-byte packing keeps these longs where Windows puts them
        [StructLayout(LayoutKind.Sequential, Pack = 4)]
        public struct WIN32_FILE_ATTRIBUTE_DATA
        {
            public uint dwFileAttributes;
            public long ftCreationTime;
            public long ftLastAccessTime;
            public long ftLastWriteTime;
            public uint nFileSizeHigh;
            public uint nFileSizeLow;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetFileAttributesExW(string fileName, int infoLevel, out WIN32_FILE_ATTRIBUTE_DATA data);

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode, Pack = 4)]
        public struct WIN32_FIND_DATAW
        {
            public uint dwFileAttributes;
            public long ftCreationTime;
            public long ftLastAccessTime;
            public long ftLastWriteTime;
            public uint nFileSizeHigh;
            public uint nFileSizeLow;
            public uint dwReserved0;
            public uint dwReserved1;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string cFileName;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 14)] public string cAlternateFileName;
        }

        public const int FindExInfoBasic = 1;
        public const int FindExSearchNameMatch = 0;
        public const int FIND_FIRST_EX_LARGE_FETCH = 2;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr FindFirstFileExW(string fileName, int infoLevel, out WIN32_FIND_DATAW findData,
            int searchOp, IntPtr searchFilter, int additionalFlags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool FindNextFileW(IntPtr findFile, out WIN32_FIND_DATAW findData);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool FindClose(IntPtr findFile);

        public static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern SafeFileHandle CreateFileW(string fileName, uint desiredAccess, uint shareMode, IntPtr securityAttributes,
            uint creationDisposition, uint flagsAndAttributes, IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool ReadFile(SafeFileHandle file, byte* buffer, int numberOfBytesToRead, out int numberOfBytesRead, IntPtr overlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool WriteFile(SafeFileHandle file, byte* buffer, int numberOfBytesToWrite, out int numberOfBytesWritten, IntPtr overlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetFileSizeEx(SafeFileHandle file, out long size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool FlushFileBuffers(SafeFileHandle file);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool SetFileInformationByHandle(SafeFileHandle file, int fileInformationClass, void* info, int bufferSize);

        public const int FileBasicInfo = 0, FileAllocationInfo = 5, FileEndOfFileInfo = 6;

        [StructLayout(LayoutKind.Sequential)]
        public struct FILE_BASIC_INFO
        {
            public long CreationTime, LastAccessTime, LastWriteTime, ChangeTime;
            public uint FileAttributes;
            uint pad;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        public struct WIN32_FIND_STREAM_DATA
        {
            public long StreamSize;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 296)] public string cStreamName;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr FindFirstStreamW(string fileName, int infoLevel, out WIN32_FIND_STREAM_DATA findStreamData, int flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool FindNextStreamW(IntPtr findStream, out WIN32_FIND_STREAM_DATA findStreamData);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetDiskFreeSpaceW(string rootPathName, out uint sectorsPerCluster, out uint bytesPerSector, out uint freeClusters, out uint totalClusters);

        [DllImport("kernel32.dll", SetLastError = true, EntryPoint = "ReadFile")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool ReadFileAt(SafeFileHandle file, byte* buffer, int numberOfBytesToRead, IntPtr numberOfBytesRead, System.Threading.NativeOverlapped* overlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool GetOverlappedResult(SafeFileHandle file, System.Threading.NativeOverlapped* overlapped, out int bytes, [MarshalAs(UnmanagedType.Bool)] bool wait);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr CreateEventW(IntPtr attributes, [MarshalAs(UnmanagedType.Bool)] bool manualReset, [MarshalAs(UnmanagedType.Bool)] bool initialState, IntPtr name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool ResetEvent(IntPtr handle);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool CloseHandle(IntPtr handle);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFilePointerEx(SafeFileHandle file, long distance, out long newPointer, int moveMethod);

        [StructLayout(LayoutKind.Sequential)]
        public struct MEMORYSTATUSEX
        {
            public uint dwLength, dwMemoryLoad;
            public ulong ullTotalPhys, ullAvailPhys, ullTotalPageFile, ullAvailPageFile, ullTotalVirtual, ullAvailVirtual, ullAvailExtendedVirtual;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX buffer);

        // where a file's data starts on the disk (no admin rights needed): read many small files in disk order
        public const uint FSCTL_GET_RETRIEVAL_POINTERS = 0x00090073;

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern unsafe bool DeviceIoControl(SafeFileHandle device, uint ioControlCode, ref long inBuffer, int inBufferSize,
            byte* outBuffer, int outBufferSize, out int bytesReturned, IntPtr overlapped);

        public const int ERROR_IO_PENDING = 997;
        public const uint FILE_FLAG_OVERLAPPED = 0x40000000;
        public const uint GENERIC_WRITE = 0x40000000;
        public const uint DELETE = 0x00010000;
        public const uint CREATE_NEW = 1;
        public const int ERROR_INVALID_PARAMETER = 87;
        public const uint FILE_ATTRIBUTE_HIDDEN = 0x2, FILE_ATTRIBUTE_SYSTEM = 0x4, FILE_ATTRIBUTE_ARCHIVE = 0x20;
        public const uint FILE_ATTRIBUTE_SPARSE_FILE = 0x200, FILE_ATTRIBUTE_COMPRESSED = 0x800, FILE_ATTRIBUTE_OFFLINE = 0x1000;
        public const uint FILE_ATTRIBUTE_NOT_CONTENT_INDEXED = 0x2000, FILE_ATTRIBUTE_ENCRYPTED = 0x4000;
        public const uint FILE_ATTRIBUTE_RECALL_ON_OPEN = 0x40000, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS = 0x400000;

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFileTime(SafeFileHandle file, ref long creationTime, ref long lastAccessTime, ref long lastWriteTime);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFileTime(SafeFileHandle file, ref long creationTime, IntPtr lastAccessTime, ref long lastWriteTime);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFileTime(IntPtr file, ref long creationTime, IntPtr lastAccessTime, IntPtr lastWriteTime);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetFileInformationByHandle(IntPtr file, int fileInformationClass, IntPtr info, int bufferSize);

        public const int FileRenameInfo = 3;
        public const uint CALLBACK_CHUNK_FINISHED = 0;
        public const uint CALLBACK_STREAM_SWITCH = 1;

        /// <summary>
        /// Renames an open file through its handle. newPath MUST be the full \\?\ path: Windows resolves a plain
        /// name against the process's current folder, not the file's folder. Never replaces an existing file.
        /// </summary>
        public static bool RenameByHandle(IntPtr handle, string newPath)
        {
            if (string.IsNullOrEmpty(newPath) || !newPath.StartsWith(@"\\?\", StringComparison.Ordinal)) return false;
            string newName = newPath;
            // FILE_RENAME_INFO { union { BOOLEAN ReplaceIfExists; DWORD Flags; }; HANDLE RootDirectory; DWORD FileNameLength; WCHAR FileName[1]; }
            int ptr = IntPtr.Size;
            int offRoot = ptr == 8 ? 8 : 4;
            int offLen = offRoot + ptr;
            int offName = offLen + 4;
            int nameBytes = newName.Length * 2;
            int size = offName + nameBytes + 2;
            IntPtr buf = Marshal.AllocHGlobal(size);
            try
            {
                for (int i = 0; i < size; i++) Marshal.WriteByte(buf, i, 0);
                Marshal.WriteInt32(buf, 0, 0);                    // ReplaceIfExists = FALSE
                Marshal.WriteIntPtr(buf, offRoot, IntPtr.Zero);
                Marshal.WriteInt32(buf, offLen, nameBytes);
                byte[] nb = Encoding.Unicode.GetBytes(newName);
                Marshal.Copy(nb, 0, buf + offName, nb.Length);
                return SetFileInformationByHandle(handle, FileRenameInfo, buf, size);
            }
            finally { Marshal.FreeHGlobal(buf); }
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetVolumePathNameW(string fileName, StringBuilder volumePathName, int bufferLength);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetVolumeInformationW(string rootPathName, StringBuilder volumeNameBuffer, int volumeNameSize,
            out uint volumeSerialNumber, out uint maximumComponentLength, out uint fileSystemFlags, StringBuilder fileSystemNameBuffer, int nFileSystemNameSize);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern uint GetDriveTypeW(string rootPathName);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetDiskFreeSpaceExW(string directoryName, out ulong freeBytesAvailable, out ulong totalNumberOfBytes, out ulong totalNumberOfFreeBytes);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetVolumeNameForVolumeMountPointW(string volumeMountPoint, StringBuilder volumeName, int bufferLength);

        // Storage queries (seek penalty, bus type)
        public const uint IOCTL_STORAGE_QUERY_PROPERTY = 0x002D1400;

        [StructLayout(LayoutKind.Sequential)]
        public struct STORAGE_PROPERTY_QUERY
        {
            public int PropertyId;
            public int QueryType;
            public byte AdditionalParameters;
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct DEVICE_SEEK_PENALTY_DESCRIPTOR
        {
            public uint Version;
            public uint Size;
            [MarshalAs(UnmanagedType.U1)] public bool IncursSeekPenalty;
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct STORAGE_DEVICE_DESCRIPTOR_HEADER
        {
            public uint Version;
            public uint Size;
            public byte DeviceType;
            public byte DeviceTypeModifier;
            [MarshalAs(UnmanagedType.U1)] public bool RemovableMedia;
            [MarshalAs(UnmanagedType.U1)] public bool CommandQueueing;
            public uint VendorIdOffset;
            public uint ProductIdOffset;
            public uint ProductRevisionOffset;
            public uint SerialNumberOffset;
            public int BusType;
            public uint RawPropertiesLength;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool DeviceIoControl(SafeFileHandle device, uint ioControlCode, ref STORAGE_PROPERTY_QUERY inBuffer, int inBufferSize,
            out DEVICE_SEEK_PENALTY_DESCRIPTOR outBuffer, int outBufferSize, out int bytesReturned, IntPtr overlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool DeviceIoControl(SafeFileHandle device, uint ioControlCode, ref STORAGE_PROPERTY_QUERY inBuffer, int inBufferSize,
            out STORAGE_DEVICE_DESCRIPTOR_HEADER outBuffer, int outBufferSize, out int bytesReturned, IntPtr overlapped);

        // which physical disk a volume sits on (two volumes on one disk share the heads)
        public const uint IOCTL_STORAGE_GET_DEVICE_NUMBER = 0x002D1080;

        [StructLayout(LayoutKind.Sequential)]
        public struct STORAGE_DEVICE_NUMBER
        {
            public int DeviceType;
            public int DeviceNumber;
            public int PartitionNumber;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool DeviceIoControl(SafeFileHandle device, uint ioControlCode, IntPtr inBuffer, int inBufferSize,
            out STORAGE_DEVICE_NUMBER outBuffer, int outBufferSize, out int bytesReturned, IntPtr overlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr VirtualAlloc(IntPtr address, UIntPtr size, uint allocationType, uint protect);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool VirtualFree(IntPtr address, UIntPtr size, uint freeType);

        public const uint MEM_COMMIT = 0x1000, MEM_RESERVE = 0x2000, MEM_RELEASE = 0x8000, PAGE_READWRITE = 0x04;

        public static string ErrorText(int code)
        {
            switch (code)
            {
                case ERROR_FILE_NOT_FOUND: return "The file is gone (it was moved or deleted).";
                case ERROR_PATH_NOT_FOUND: return "The folder is gone or can't be reached.";
                case ERROR_ACCESS_DENIED: return "Access denied.";
                case ERROR_SHARING_VIOLATION: return "The file is open in another program.";
                case ERROR_LOCK_VIOLATION: return "Part of the file is locked by another program.";
                case ERROR_DISK_FULL:
                case ERROR_HANDLE_DISK_FULL: return "The destination is full.";
                case ERROR_BAD_NETPATH:
                case ERROR_BAD_NET_NAME:
                case ERROR_NETNAME_DELETED:
                case ERROR_UNEXP_NET_ERR:
                case ERROR_NETWORK_UNREACHABLE:
                case ERROR_HOST_UNREACHABLE:
                case ERROR_CONNECTION_ABORTED:
                case ERROR_NOT_CONNECTED:
                case ERROR_SEM_TIMEOUT: return "The network connection dropped.";
                case ERROR_INVALID_NAME: return "The name isn't allowed at the destination.";
                case ERROR_FILENAME_EXCED_RANGE: return "The path is too long for the destination.";
                case ERROR_CRC:
                case ERROR_IO_DEVICE: return "Read error on the disk (bad sector?).";
                case ERROR_REQUEST_ABORTED: return "Stopped.";
            }
            string msg = new System.ComponentModel.Win32Exception(code).Message;
            return msg + " (" + code + ")";
        }
    }
}
