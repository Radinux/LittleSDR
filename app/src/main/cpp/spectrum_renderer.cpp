// Vulkan spectrum + waterfall renderer.
//
// Data path: the DSP thread writes FFT lines into engine::fftHistory() (CPU ring).
// Each frame, new lines are copied into a host-visible storage buffer that the
// fragment shader indexes directly (no image uploads / layout transitions).
// The spectrum trace is a triangle strip generated in the vertex shader from the
// latest (smoothed) line. Colors use the Turbo colormap computed in the shader.
#include "spectrum_renderer.h"
#include "engine.h"

#include <vulkan/vulkan.h>
#include <vulkan/vulkan_android.h>
#include <android/log.h>
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>
#include <vector>

#define LOG_TAG "LittleSDR-VK"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, LOG_TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, LOG_TAG, __VA_ARGS__)
#define VK_CHECK(x)                                                    \
    do {                                                               \
        VkResult r_ = (x);                                             \
        if (r_ != VK_SUCCESS) {                                        \
            LOGE("%s failed: %d (%s:%d)", #x, r_, __FILE__, __LINE__); \
            return false;                                              \
        }                                                              \
    } while (0)

namespace {
    const uint32_t kWaterfallVert[] = {
#include "waterfall_vert.spv.h"
    };
    const uint32_t kWaterfallFrag[] = {
#include "waterfall_frag.spv.h"
    };
    const uint32_t kSpectrumVert[] = {
#include "spectrum_vert.spv.h"
    };
    const uint32_t kSpectrumFrag[] = {
#include "spectrum_frag.spv.h"
    };

    // Must match shaders/common.glsl
    struct PushConstants {
        float binStart, binCount, dbMin, dbMax;
        int32_t bins, rows, newest, filled;
        float widthPx, heightPx, offY, lineW;
        float fillColor[4];
        float lineColor[4];
        int32_t palette;
        int32_t lineOffset;
        int32_t pad[2];
    };
    static_assert(sizeof(PushConstants) == 96, "push constant layout");

    constexpr int FRAMES = 2;

    struct HostBuffer {
        VkBuffer buf = VK_NULL_HANDLE;
        VkDeviceMemory mem = VK_NULL_HANDLE;
        void* mapped = nullptr;
        VkDeviceSize size = 0;
    };

    class Renderer {
    public:
        bool start(ANativeWindow* w) {
            window = w;
            running = true;
            thread = std::thread([this] { run(); });
            return true;
        }

        void stop() {
            running = false;
            if (thread.joinable()) { thread.join(); }
            if (window) { ANativeWindow_release(window); window = nullptr; }
        }

        void requestResize() { needRecreate = true; dirty = true; }
        void setView(const spectrum::ViewParams& p) {
            std::lock_guard<std::mutex> l(viewMtx);
            params = p;
            dirty = true;
        }
        spectrum::ViewParams view() {
            std::lock_guard<std::mutex> l(viewMtx);
            return params;
        }

    private:
        // ------------------------------------------------------------ thread
        void run() {
            if (!initVulkan()) {
                LOGE("Vulkan init failed");
                destroyVulkan();
                return;
            }
            LOGI("Vulkan renderer running (%ux%u)", extent.width, extent.height);
            auto lastDraw = std::chrono::steady_clock::now();
            while (running) {
                if (needRecreate) {
                    needRecreate = false;
                    if (!recreateSwapchain()) { break; }
                }
                bool newData = syncData();
                auto now = std::chrono::steady_clock::now();
                // Redraw on new FFT lines, parameter changes, or at least every 500 ms
                if (!newData && !dirty && now - lastDraw < std::chrono::milliseconds(500)) {
                    std::this_thread::sleep_for(std::chrono::milliseconds(4));
                    continue;
                }
                dirty = false;
                lastDraw = now;
                if (extent.width == 0 || extent.height == 0) {
                    std::this_thread::sleep_for(std::chrono::milliseconds(16));
                    continue;
                }
                if (!drawFrame()) { break; }
            }
            if (device) { vkDeviceWaitIdle(device); }
            destroyVulkan();
            LOGI("Vulkan renderer stopped");
        }

        // ------------------------------------------------------------ setup
        // The instance, device, pipelines and FFT buffers live for the whole process (created on
        // the first attach); each attach only creates a surface, a swapchain and sync objects.
        // Recreating the device on every navigation was wasteful and stressed emulator drivers.
        bool initVulkan() {
            if (!ctxReady && !instance) {
            VkApplicationInfo app{ VK_STRUCTURE_TYPE_APPLICATION_INFO };
            app.pApplicationName = "LittleSDR";
            app.apiVersion = VK_API_VERSION_1_0;
            const char* instExt[] = { VK_KHR_SURFACE_EXTENSION_NAME, VK_KHR_ANDROID_SURFACE_EXTENSION_NAME };
            VkInstanceCreateInfo ici{ VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
            ici.pApplicationInfo = &app;
            ici.enabledExtensionCount = 2;
            ici.ppEnabledExtensionNames = instExt;
            VK_CHECK(vkCreateInstance(&ici, nullptr, &instance));
            }

            VkAndroidSurfaceCreateInfoKHR sci{ VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR };
            sci.window = window;
            VK_CHECK(vkCreateAndroidSurfaceKHR(instance, &sci, nullptr, &surface));

            if (ctxReady) {
                VkBool32 present = VK_FALSE;
                vkGetPhysicalDeviceSurfaceSupportKHR(phys, queueFamily, surface, &present);
                if (!present) { LOGE("queue can't present to the new surface"); return false; }
            } else {
            uint32_t n = 0;
            vkEnumeratePhysicalDevices(instance, &n, nullptr);
            if (n == 0) { LOGE("no Vulkan device"); return false; }
            std::vector<VkPhysicalDevice> devs(n);
            vkEnumeratePhysicalDevices(instance, &n, devs.data());
            for (auto d : devs) {
                uint32_t qn = 0;
                vkGetPhysicalDeviceQueueFamilyProperties(d, &qn, nullptr);
                std::vector<VkQueueFamilyProperties> qp(qn);
                vkGetPhysicalDeviceQueueFamilyProperties(d, &qn, qp.data());
                for (uint32_t q = 0; q < qn; q++) {
                    VkBool32 present = VK_FALSE;
                    vkGetPhysicalDeviceSurfaceSupportKHR(d, q, surface, &present);
                    if ((qp[q].queueFlags & VK_QUEUE_GRAPHICS_BIT) && present) {
                        phys = d;
                        queueFamily = q;
                        break;
                    }
                }
                if (phys) { break; }
            }
            if (!phys) { LOGE("no graphics+present queue"); return false; }

            float prio = 1.0f;
            VkDeviceQueueCreateInfo qci{ VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO };
            qci.queueFamilyIndex = queueFamily;
            qci.queueCount = 1;
            qci.pQueuePriorities = &prio;
            const char* devExt[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };
            VkDeviceCreateInfo dci{ VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
            dci.queueCreateInfoCount = 1;
            dci.pQueueCreateInfos = &qci;
            dci.enabledExtensionCount = 1;
            dci.ppEnabledExtensionNames = devExt;
            VK_CHECK(vkCreateDevice(phys, &dci, nullptr, &device));
            vkGetDeviceQueue(device, queueFamily, 0, &queue);

            // Surface format (UNORM: colormap values are used as-is)
            uint32_t fn = 0;
            vkGetPhysicalDeviceSurfaceFormatsKHR(phys, surface, &fn, nullptr);
            std::vector<VkSurfaceFormatKHR> fmts(fn);
            vkGetPhysicalDeviceSurfaceFormatsKHR(phys, surface, &fn, fmts.data());
            format = fmts.empty() ? VK_FORMAT_R8G8B8A8_UNORM : fmts[0].format;
            for (auto& f : fmts) {
                if (f.format == VK_FORMAT_R8G8B8A8_UNORM || f.format == VK_FORMAT_B8G8R8A8_UNORM) { format = f.format; break; }
            }

            if (!createRenderPass() || !createPipelines()) { return false; }

            VkDescriptorPoolSize ps{ VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 2 };
            VkDescriptorPoolCreateInfo dpci{ VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO };
            dpci.maxSets = 1;
            dpci.poolSizeCount = 1;
            dpci.pPoolSizes = &ps;
            VK_CHECK(vkCreateDescriptorPool(device, &dpci, nullptr, &descPool));
            VkDescriptorSetAllocateInfo dsai{ VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO };
            dsai.descriptorPool = descPool;
            dsai.descriptorSetCount = 1;
            dsai.pSetLayouts = &descLayout;
            VK_CHECK(vkAllocateDescriptorSets(device, &dsai, &descSet));
            ctxReady = true;
            LOGI("Vulkan context created");
            }

            // Per-surface objects
            VkCommandPoolCreateInfo cpci{ VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
            cpci.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
            cpci.queueFamilyIndex = queueFamily;
            VK_CHECK(vkCreateCommandPool(device, &cpci, nullptr, &cmdPool));
            VkCommandBufferAllocateInfo cbai{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
            cbai.commandPool = cmdPool;
            cbai.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
            cbai.commandBufferCount = FRAMES;
            VK_CHECK(vkAllocateCommandBuffers(device, &cbai, cmd));
            for (int i = 0; i < FRAMES; i++) {
                VkFenceCreateInfo fci{ VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
                fci.flags = VK_FENCE_CREATE_SIGNALED_BIT;
                VK_CHECK(vkCreateFence(device, &fci, nullptr, &fences[i]));
                VkSemaphoreCreateInfo semci{ VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };
                VK_CHECK(vkCreateSemaphore(device, &semci, nullptr, &acquireSem[i]));
            }

            return createSwapchain();
        }

        bool createRenderPass() {
            VkAttachmentDescription att{};
            att.format = format;
            att.samples = VK_SAMPLE_COUNT_1_BIT;
            att.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
            att.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
            att.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
            att.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
            att.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
            att.finalLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
            VkAttachmentReference ref{ 0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL };
            VkSubpassDescription sub{};
            sub.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
            sub.colorAttachmentCount = 1;
            sub.pColorAttachments = &ref;
            VkSubpassDependency dep{};
            dep.srcSubpass = VK_SUBPASS_EXTERNAL;
            dep.dstSubpass = 0;
            dep.srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            dep.dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            dep.dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
            VkRenderPassCreateInfo rpci{ VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
            rpci.attachmentCount = 1;
            rpci.pAttachments = &att;
            rpci.subpassCount = 1;
            rpci.pSubpasses = &sub;
            rpci.dependencyCount = 1;
            rpci.pDependencies = &dep;
            VK_CHECK(vkCreateRenderPass(device, &rpci, nullptr, &renderPass));
            return true;
        }

        VkShaderModule shader(const uint32_t* code, size_t bytes) {
            VkShaderModuleCreateInfo ci{ VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
            ci.codeSize = bytes;
            ci.pCode = code;
            VkShaderModule m = VK_NULL_HANDLE;
            if (vkCreateShaderModule(device, &ci, nullptr, &m) != VK_SUCCESS) { LOGE("shader module failed"); }
            return m;
        }

        bool createPipelines() {
            VkDescriptorSetLayoutBinding b[2]{};
            for (int i = 0; i < 2; i++) {
                b[i].binding = i;
                b[i].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
                b[i].descriptorCount = 1;
                b[i].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;
            }
            VkDescriptorSetLayoutCreateInfo dslci{ VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO };
            dslci.bindingCount = 2;
            dslci.pBindings = b;
            VK_CHECK(vkCreateDescriptorSetLayout(device, &dslci, nullptr, &descLayout));

            VkPushConstantRange pcr{ VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof(PushConstants) };
            VkPipelineLayoutCreateInfo plci{ VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
            plci.setLayoutCount = 1;
            plci.pSetLayouts = &descLayout;
            plci.pushConstantRangeCount = 1;
            plci.pPushConstantRanges = &pcr;
            VK_CHECK(vkCreatePipelineLayout(device, &plci, nullptr, &pipeLayout));

            wfPipe = makePipeline(kWaterfallVert, sizeof(kWaterfallVert), kWaterfallFrag, sizeof(kWaterfallFrag),
                                  VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST, false);
            specPipe = makePipeline(kSpectrumVert, sizeof(kSpectrumVert), kSpectrumFrag, sizeof(kSpectrumFrag),
                                    VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP, true);
            return wfPipe && specPipe;
        }

        VkPipeline makePipeline(const uint32_t* vs, size_t vsz, const uint32_t* fs, size_t fsz,
                                VkPrimitiveTopology topo, bool blend) {
            VkShaderModule vm = shader(vs, vsz), fm = shader(fs, fsz);
            VkPipelineShaderStageCreateInfo st[2]{};
            st[0] = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_VERTEX_BIT, vm, "main", nullptr };
            st[1] = { VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_FRAGMENT_BIT, fm, "main", nullptr };
            VkPipelineVertexInputStateCreateInfo vi{ VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
            VkPipelineInputAssemblyStateCreateInfo ia{ VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO };
            ia.topology = topo;
            VkPipelineViewportStateCreateInfo vp{ VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };
            vp.viewportCount = 1;
            vp.scissorCount = 1;
            VkPipelineRasterizationStateCreateInfo rs{ VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO };
            rs.polygonMode = VK_POLYGON_MODE_FILL;
            rs.cullMode = VK_CULL_MODE_NONE;
            rs.lineWidth = 1.0f;
            VkPipelineMultisampleStateCreateInfo ms{ VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO };
            ms.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
            VkPipelineColorBlendAttachmentState cba{};
            cba.colorWriteMask = 0xF;
            if (blend) {
                cba.blendEnable = VK_TRUE;
                cba.srcColorBlendFactor = VK_BLEND_FACTOR_SRC_ALPHA;
                cba.dstColorBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
                cba.colorBlendOp = VK_BLEND_OP_ADD;
                cba.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
                cba.dstAlphaBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
                cba.alphaBlendOp = VK_BLEND_OP_ADD;
            }
            VkPipelineColorBlendStateCreateInfo cb{ VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };
            cb.attachmentCount = 1;
            cb.pAttachments = &cba;
            VkDynamicState dyn[] = { VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR };
            VkPipelineDynamicStateCreateInfo ds{ VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
            ds.dynamicStateCount = 2;
            ds.pDynamicStates = dyn;
            VkGraphicsPipelineCreateInfo gp{ VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
            gp.stageCount = 2;
            gp.pStages = st;
            gp.pVertexInputState = &vi;
            gp.pInputAssemblyState = &ia;
            gp.pViewportState = &vp;
            gp.pRasterizationState = &rs;
            gp.pMultisampleState = &ms;
            gp.pColorBlendState = &cb;
            gp.pDynamicState = &ds;
            gp.layout = pipeLayout;
            gp.renderPass = renderPass;
            VkPipeline p = VK_NULL_HANDLE;
            if (vkCreateGraphicsPipelines(device, VK_NULL_HANDLE, 1, &gp, nullptr, &p) != VK_SUCCESS) { LOGE("pipeline failed"); p = VK_NULL_HANDLE; }
            vkDestroyShaderModule(device, vm, nullptr);
            vkDestroyShaderModule(device, fm, nullptr);
            return p;
        }

        bool createSwapchain() {
            VkSurfaceCapabilitiesKHR caps;
            VK_CHECK(vkGetPhysicalDeviceSurfaceCapabilitiesKHR(phys, surface, &caps));
            extent = caps.currentExtent;
            if (extent.width == 0xFFFFFFFF) {
                extent = { (uint32_t)ANativeWindow_getWidth(window), (uint32_t)ANativeWindow_getHeight(window) };
            }
            if (extent.width == 0 || extent.height == 0) { return true; } // minimized, retry later

            uint32_t count = std::max(caps.minImageCount + 1, 3u);
            if (caps.maxImageCount > 0) { count = std::min(count, caps.maxImageCount); }
            VkCompositeAlphaFlagBitsKHR alpha = VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR;
            for (auto a : { VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR, VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR,
                            VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR, VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR }) {
                if (caps.supportedCompositeAlpha & a) { alpha = a; break; }
            }
            VkSwapchainCreateInfoKHR ci{ VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };
            ci.surface = surface;
            ci.minImageCount = count;
            ci.imageFormat = format;
            ci.imageColorSpace = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
            ci.imageExtent = extent;
            ci.imageArrayLayers = 1;
            ci.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
            ci.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
            ci.preTransform = (caps.supportedTransforms & VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR)
                                  ? VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR : caps.currentTransform;
            ci.compositeAlpha = alpha;
            ci.presentMode = VK_PRESENT_MODE_FIFO_KHR;
            ci.clipped = VK_TRUE;
            ci.oldSwapchain = swapchain;
            VkSwapchainKHR newSc = VK_NULL_HANDLE;
            VK_CHECK(vkCreateSwapchainKHR(device, &ci, nullptr, &newSc));
            destroySwapchainResources();
            if (swapchain) { vkDestroySwapchainKHR(device, swapchain, nullptr); }
            swapchain = newSc;

            uint32_t n = 0;
            vkGetSwapchainImagesKHR(device, swapchain, &n, nullptr);
            images.resize(n);
            vkGetSwapchainImagesKHR(device, swapchain, &n, images.data());
            views.resize(n);
            fbs.resize(n);
            renderSem.resize(n);
            for (uint32_t i = 0; i < n; i++) {
                VkImageViewCreateInfo ivci{ VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
                ivci.image = images[i];
                ivci.viewType = VK_IMAGE_VIEW_TYPE_2D;
                ivci.format = format;
                ivci.subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 };
                VK_CHECK(vkCreateImageView(device, &ivci, nullptr, &views[i]));
                VkFramebufferCreateInfo fbci{ VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
                fbci.renderPass = renderPass;
                fbci.attachmentCount = 1;
                fbci.pAttachments = &views[i];
                fbci.width = extent.width;
                fbci.height = extent.height;
                fbci.layers = 1;
                VK_CHECK(vkCreateFramebuffer(device, &fbci, nullptr, &fbs[i]));
                VkSemaphoreCreateInfo semci{ VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };
                VK_CHECK(vkCreateSemaphore(device, &semci, nullptr, &renderSem[i]));
            }
            return true;
        }

        bool recreateSwapchain() {
            vkDeviceWaitIdle(device);
            return createSwapchain();
        }

        void destroySwapchainResources() {
            for (auto fb : fbs) { vkDestroyFramebuffer(device, fb, nullptr); }
            for (auto v : views) { vkDestroyImageView(device, v, nullptr); }
            for (auto s : renderSem) { vkDestroySemaphore(device, s, nullptr); }
            fbs.clear();
            views.clear();
            renderSem.clear();
            images.clear();
        }

        // ------------------------------------------------------------ buffers
        bool createHostBuffer(HostBuffer& hb, VkDeviceSize size) {
            destroyHostBuffer(hb);
            VkBufferCreateInfo bci{ VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
            bci.size = size;
            bci.usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
            bci.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
            VK_CHECK(vkCreateBuffer(device, &bci, nullptr, &hb.buf));
            VkMemoryRequirements req;
            vkGetBufferMemoryRequirements(device, hb.buf, &req);
            VkPhysicalDeviceMemoryProperties mp;
            vkGetPhysicalDeviceMemoryProperties(phys, &mp);
            uint32_t type = UINT32_MAX;
            const VkMemoryPropertyFlags want = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
            for (uint32_t i = 0; i < mp.memoryTypeCount; i++) {
                if ((req.memoryTypeBits & (1u << i)) && (mp.memoryTypes[i].propertyFlags & want) == want) { type = i; break; }
            }
            if (type == UINT32_MAX) { LOGE("no host-coherent memory"); return false; }
            VkMemoryAllocateInfo mai{ VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
            mai.allocationSize = req.size;
            mai.memoryTypeIndex = type;
            VK_CHECK(vkAllocateMemory(device, &mai, nullptr, &hb.mem));
            VK_CHECK(vkBindBufferMemory(device, hb.buf, hb.mem, 0));
            VK_CHECK(vkMapMemory(device, hb.mem, 0, size, 0, &hb.mapped));
            hb.size = size;
            return true;
        }

        void destroyHostBuffer(HostBuffer& hb) {
            if (hb.mapped) { vkUnmapMemory(device, hb.mem); }
            if (hb.buf) { vkDestroyBuffer(device, hb.buf, nullptr); }
            if (hb.mem) { vkFreeMemory(device, hb.mem, nullptr); }
            hb = HostBuffer{};
        }

        // Copies new FFT lines into the GPU-visible ring. Returns true if anything changed.
        bool syncData() {
            auto& h = engine::fftHistory();
            std::lock_guard<std::mutex> l(h.mtx);
            if (h.bins <= 0) { return false; }
            if (h.generation != gpuGeneration) {
                vkDeviceWaitIdle(device);
                if (!createHostBuffer(linesBuf, (VkDeviceSize)h.bins * h.rows * sizeof(float)) ||
                    !createHostBuffer(latestBuf, (VkDeviceSize)h.bins * 2 * sizeof(float))) { // trace + peak
                    return false;
                }
                VkDescriptorBufferInfo bi[2] = { { linesBuf.buf, 0, VK_WHOLE_SIZE }, { latestBuf.buf, 0, VK_WHOLE_SIZE } };
                VkWriteDescriptorSet w[2]{};
                for (int i = 0; i < 2; i++) {
                    w[i] = { VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET };
                    w[i].dstSet = descSet;
                    w[i].dstBinding = i;
                    w[i].descriptorCount = 1;
                    w[i].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
                    w[i].pBufferInfo = &bi[i];
                }
                vkUpdateDescriptorSets(device, 2, w, 0, nullptr);
                gpuGeneration = h.generation;
                bins = h.bins;
                rows = h.rows;
                memcpy(linesBuf.mapped, h.lines.data(), h.lines.size() * sizeof(float));
                copied = h.written;
                buffersReady = true;
            }
            else if (h.written != copied) {
                uint64_t n = h.written - copied;
                if (n >= (uint64_t)rows) {
                    memcpy(linesBuf.mapped, h.lines.data(), h.lines.size() * sizeof(float));
                }
                else {
                    for (uint64_t r = copied; r < h.written; r++) {
                        size_t off = (size_t)(r % rows) * bins;
                        memcpy((float*)linesBuf.mapped + off, h.lines.data() + off, bins * sizeof(float));
                    }
                }
                copied = h.written;
            }
            else {
                return false;
            }
            memcpy(latestBuf.mapped, h.latest.data(), bins * sizeof(float));
            peakHold = h.peakHold;
            if (peakHold) { memcpy((float*)latestBuf.mapped + bins, h.peak.data(), bins * sizeof(float)); }
            newest = h.written > 0 ? (int)((h.written - 1) % rows) : 0;
            filled = (int)std::min<uint64_t>(h.written, rows);
            return true;
        }

        // ------------------------------------------------------------ frame
        bool drawFrame() {
            if (!swapchain || !buffersReady) { return true; }
            // Bounded waits (100 ms): stop() must never hang on a surface being destroyed
            constexpr uint64_t TIMEOUT_NS = 100ull * 1000 * 1000;
            if (vkWaitForFences(device, 1, &fences[frame], VK_TRUE, TIMEOUT_NS) == VK_TIMEOUT) { dirty = true; return true; }
            uint32_t idx = 0;
            VkResult r = vkAcquireNextImageKHR(device, swapchain, TIMEOUT_NS, acquireSem[frame], VK_NULL_HANDLE, &idx);
            if (r == VK_TIMEOUT || r == VK_NOT_READY) { dirty = true; return true; }
            if (r == VK_ERROR_OUT_OF_DATE_KHR) { needRecreate = true; return true; }
            if (r == VK_ERROR_SURFACE_LOST_KHR) { return false; }
            if (r != VK_SUCCESS && r != VK_SUBOPTIMAL_KHR) { LOGE("acquire %d", r); return false; }
            vkResetFences(device, 1, &fences[frame]);

            spectrum::ViewParams p = view();
            PushConstants pc{};
            double sr = engine::sampleRate();
            double viewBw = (p.viewBandwidth > 0 && p.viewBandwidth < sr) ? p.viewBandwidth : sr;
            pc.binCount = (float)(bins * viewBw / sr);
            pc.binStart = (float)(bins * 0.5 + (p.viewOffset - viewBw / 2.0) / sr * bins);
            pc.dbMin = p.dbMin;
            pc.dbMax = p.dbMax;
            pc.bins = bins;
            pc.rows = rows;
            pc.newest = newest;
            pc.filled = filled;
            pc.lineW = p.lineWidthPx;
            memcpy(pc.fillColor, p.fillColor, sizeof(pc.fillColor));
            memcpy(pc.lineColor, p.lineColor, sizeof(pc.lineColor));
            pc.palette = p.palette;

            const float W = (float)extent.width, H = (float)extent.height;
            const float specH = std::floor(H * std::clamp(p.spectrumFraction, 0.1f, 0.9f));
            const float wfH = H - specH;

            VkCommandBuffer cb = cmd[frame];
            vkResetCommandBuffer(cb, 0);
            VkCommandBufferBeginInfo bi{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
            bi.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
            vkBeginCommandBuffer(cb, &bi);
            VkClearValue clear;
            memcpy(clear.color.float32, p.background, sizeof(p.background));
            VkRenderPassBeginInfo rp{ VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
            rp.renderPass = renderPass;
            rp.framebuffer = fbs[idx];
            rp.renderArea = { { 0, 0 }, extent };
            rp.clearValueCount = 1;
            rp.pClearValues = &clear;
            vkCmdBeginRenderPass(cb, &rp, VK_SUBPASS_CONTENTS_INLINE);
            vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeLayout, 0, 1, &descSet, 0, nullptr);
            const VkShaderStageFlags stages = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;

            // Waterfall (bottom)
            if (wfH >= 1.0f) {
                VkViewport vp{ 0, specH, W, wfH, 0, 1 };
                VkRect2D sc{ { 0, (int32_t)specH }, { extent.width, (uint32_t)wfH } };
                vkCmdSetViewport(cb, 0, 1, &vp);
                vkCmdSetScissor(cb, 0, 1, &sc);
                pc.widthPx = W;
                pc.heightPx = wfH;
                pc.offY = specH;
                vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, wfPipe);
                vkCmdPushConstants(cb, pipeLayout, stages, 0, sizeof(pc), &pc);
                vkCmdDraw(cb, 6, 1, 0, 0);
            }

            // Spectrum trace (top)
            {
                VkViewport vp{ 0, 0, W, specH, 0, 1 };
                VkRect2D sc{ { 0, 0 }, { extent.width, (uint32_t)specH } };
                vkCmdSetViewport(cb, 0, 1, &vp);
                vkCmdSetScissor(cb, 0, 1, &sc);
                pc.widthPx = W;
                pc.heightPx = specH;
                pc.offY = 0;
                vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_GRAPHICS, specPipe);
                vkCmdPushConstants(cb, pipeLayout, stages, 0, sizeof(pc), &pc);
                vkCmdDraw(cb, (uint32_t)W * 2, 1, 0, 0);

                // Peak-hold line: same geometry, transparent fill, dimmer line
                if (peakHold) {
                    PushConstants pk = pc;
                    pk.lineOffset = bins;
                    pk.fillColor[3] = 0.0f;
                    pk.lineColor[3] = 0.55f;
                    pk.lineW = pc.lineW * 0.75f;
                    vkCmdPushConstants(cb, pipeLayout, stages, 0, sizeof(pk), &pk);
                    vkCmdDraw(cb, (uint32_t)W * 2, 1, 0, 0);
                }
            }

            vkCmdEndRenderPass(cb);
            vkEndCommandBuffer(cb);

            VkPipelineStageFlags waitStage = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            VkSubmitInfo si{ VK_STRUCTURE_TYPE_SUBMIT_INFO };
            si.waitSemaphoreCount = 1;
            si.pWaitSemaphores = &acquireSem[frame];
            si.pWaitDstStageMask = &waitStage;
            si.commandBufferCount = 1;
            si.pCommandBuffers = &cb;
            si.signalSemaphoreCount = 1;
            si.pSignalSemaphores = &renderSem[idx];
            VK_CHECK(vkQueueSubmit(queue, 1, &si, fences[frame]));

            VkPresentInfoKHR pi{ VK_STRUCTURE_TYPE_PRESENT_INFO_KHR };
            pi.waitSemaphoreCount = 1;
            pi.pWaitSemaphores = &renderSem[idx];
            pi.swapchainCount = 1;
            pi.pSwapchains = &swapchain;
            pi.pImageIndices = &idx;
            r = vkQueuePresentKHR(queue, &pi);
            if (r == VK_ERROR_OUT_OF_DATE_KHR || r == VK_SUBOPTIMAL_KHR) { needRecreate = true; }
            else if (r == VK_ERROR_SURFACE_LOST_KHR) { return false; }
            frame = (frame + 1) % FRAMES;
            return true;
        }

        // ------------------------------------------------------------ teardown
        void destroyVulkan() {
            if (device) {
                vkDeviceWaitIdle(device);
                destroySwapchainResources();
                if (swapchain) { vkDestroySwapchainKHR(device, swapchain, nullptr); }
                for (int i = 0; i < FRAMES; i++) {
                    if (fences[i]) { vkDestroyFence(device, fences[i], nullptr); }
                    if (acquireSem[i]) { vkDestroySemaphore(device, acquireSem[i], nullptr); }
                }
                if (cmdPool) { vkDestroyCommandPool(device, cmdPool, nullptr); }
            }
            if (surface) { vkDestroySurfaceKHR(instance, surface, nullptr); }
            swapchain = VK_NULL_HANDLE;
            surface = VK_NULL_HANDLE;
            cmdPool = VK_NULL_HANDLE;
        }

    private:
        ANativeWindow* window = nullptr;
        std::thread thread;
        std::atomic<bool> running{ false };
        std::atomic<bool> needRecreate{ false };
        std::atomic<bool> dirty{ true };
        std::mutex viewMtx;
        spectrum::ViewParams params;

        // ---- shared, process lifetime
        static inline bool ctxReady = false;
        static inline VkInstance instance = VK_NULL_HANDLE;
        static inline VkPhysicalDevice phys = VK_NULL_HANDLE;
        static inline uint32_t queueFamily = 0;
        static inline VkDevice device = VK_NULL_HANDLE;
        static inline VkQueue queue = VK_NULL_HANDLE;
        static inline VkFormat format = VK_FORMAT_R8G8B8A8_UNORM;
        static inline VkRenderPass renderPass = VK_NULL_HANDLE;
        static inline VkDescriptorSetLayout descLayout = VK_NULL_HANDLE;
        static inline VkPipelineLayout pipeLayout = VK_NULL_HANDLE;
        static inline VkPipeline wfPipe = VK_NULL_HANDLE, specPipe = VK_NULL_HANDLE;
        static inline VkDescriptorPool descPool = VK_NULL_HANDLE;
        static inline VkDescriptorSet descSet = VK_NULL_HANDLE;
        static inline HostBuffer linesBuf, latestBuf;
        static inline uint64_t gpuGeneration = UINT64_MAX;
        static inline uint64_t copied = 0;
        static inline bool buffersReady = false;
        static inline int bins = 0, rows = 0, newest = 0, filled = 0;
        static inline bool peakHold = false;

        // ---- per surface
        VkSurfaceKHR surface = VK_NULL_HANDLE;
        VkExtent2D extent{ 0, 0 };
        VkSwapchainKHR swapchain = VK_NULL_HANDLE;
        std::vector<VkImage> images;
        std::vector<VkImageView> views;
        std::vector<VkFramebuffer> fbs;
        std::vector<VkSemaphore> renderSem;
        VkCommandPool cmdPool = VK_NULL_HANDLE;
        VkCommandBuffer cmd[FRAMES]{};
        VkFence fences[FRAMES]{};
        VkSemaphore acquireSem[FRAMES]{};
        int frame = 0;
    };

    std::mutex g_mtx;
    Renderer* g_renderer = nullptr;
    spectrum::ViewParams g_params;
}

namespace spectrum {
    bool attach(ANativeWindow* window) {
        std::lock_guard<std::mutex> l(g_mtx);
        if (g_renderer) {
            g_renderer->stop();
            delete g_renderer;
        }
        g_renderer = new Renderer();
        g_renderer->setView(g_params);
        return g_renderer->start(window);
    }

    void resize(int, int) {
        std::lock_guard<std::mutex> l(g_mtx);
        if (g_renderer) { g_renderer->requestResize(); }
    }

    void detach() {
        std::lock_guard<std::mutex> l(g_mtx);
        if (!g_renderer) { return; }
        g_renderer->stop();
        delete g_renderer;
        g_renderer = nullptr;
    }

    void setView(const ViewParams& p) {
        std::lock_guard<std::mutex> l(g_mtx);
        g_params = p;
        if (g_renderer) { g_renderer->setView(p); }
    }

    ViewParams view() {
        std::lock_guard<std::mutex> l(g_mtx);
        return g_params;
    }
}
