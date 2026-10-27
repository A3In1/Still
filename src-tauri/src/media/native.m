#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <AVFoundation/AVFoundation.h>
#import <Vision/Vision.h>

// Each call owns its decoder and autorelease pool; no shared codec state.
static bool writePreview(CGImageRef image, const char *output) {
    // Reduced still previews use PNG as the temporary bridge so transparent
    // PNG/WebP inputs retain alpha before Rust encodes the final WebP.
    bool png = [[@(output) pathExtension] caseInsensitiveCompare:@"png"] == NSOrderedSame;
    if (!image) return false;
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(NULL, CGImageGetWidth(image), CGImageGetHeight(image), 8, 0, space, (CGBitmapInfo)(png ? kCGImageAlphaPremultipliedLast : kCGImageAlphaNoneSkipLast));
    CGColorSpaceRelease(space);
    if (!context) return false;
    CGContextDrawImage(context, CGRectMake(0, 0, CGImageGetWidth(image), CGImageGetHeight(image)), image);
    CGImageRef converted = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    CGImageDestinationRef destination = CGImageDestinationCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(output)], png ? CFSTR("public.png") : CFSTR("public.jpeg"), 1, NULL);
    if (!destination) { CGImageRelease(converted); return false; }
    CGImageDestinationAddImage(destination, converted, (__bridge CFDictionaryRef)@{(__bridge NSString *)kCGImageDestinationLossyCompressionQuality: @0.92});
    bool ok = CGImageDestinationFinalize(destination);
    CFRelease(destination); CGImageRelease(converted);
    return ok;
}
int gphotos_decode(const char *path, const char *output, unsigned edge) {
    @autoreleasepool {
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(path)], NULL);
        if (!source) return 0;
        NSMutableDictionary *options = [@{(__bridge NSString *)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
            (__bridge NSString *)kCGImageSourceCreateThumbnailWithTransform: @YES,
            (__bridge NSString *)kCGImageSourceThumbnailMaxPixelSize: @(edge ? edge : 32768)} mutableCopy];
        if (@available(macOS 14.0, *)) options[(NSString *)kCGImageSourceDecodeRequest]=(NSString *)kCGImageSourceDecodeToSDR;
        CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
        bool ok = writePreview(image, output);
        if (image) CGImageRelease(image);
        CFRelease(source);
        return ok;
    }
}
int gphotos_frame(const char *path, const char *output, long long ms, unsigned edge) {
    @autoreleasepool {
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:@(path)] options:nil];
        AVAssetImageGenerator *generator = [AVAssetImageGenerator assetImageGeneratorWithAsset:asset];
        generator.appliesPreferredTrackTransform = YES;
        generator.maximumSize = CGSizeMake(edge, edge);
        generator.requestedTimeToleranceBefore = kCMTimeZero;
        generator.requestedTimeToleranceAfter = kCMTimeZero;
        CGImageRef image = [generator copyCGImageAtTime:CMTimeMake(ms, 1000) actualTime:NULL error:NULL];
        bool ok = writePreview(image, output);
        if (image) CGImageRelease(image);
        return ok;
    }
}
char *gphotos_metadata(const char *path, int video) {
    @autoreleasepool {
        NSMutableDictionary *result = [NSMutableDictionary dictionary];
        NSURL *url = [NSURL fileURLWithPath:@(path)];
        if (video) {
            AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
            if(asset.creationDate.dateValue) result[@"taken_at"] = @((long long)asset.creationDate.dateValue.timeIntervalSince1970);
            double duration = CMTimeGetSeconds(asset.duration);
            if (isfinite(duration)) result[@"anim_duration_ms"] = @((long long)(duration * 1000));
            AVAssetTrack *track = [[asset tracksWithMediaType:AVMediaTypeVideo] firstObject];
            if (track) {
                CGSize size = CGSizeApplyAffineTransform(track.naturalSize, track.preferredTransform);
                result[@"width"] = @((int)fabs(size.width)); result[@"height"] = @((int)fabs(size.height));
            }
            for (NSString *format in asset.availableMetadataFormats) {
                for (AVMetadataItem *item in [asset metadataForFormat:format]) {
                    NSString *key = [item.key description];
                    if ([key containsString:@"content.identifier"]) result[@"content_id"] = item.stringValue ?: @"";
                    if ([key containsString:@"creationdate"]) {
                        NSDate *date = [[[NSISO8601DateFormatter alloc] init] dateFromString:item.stringValue];
                        if (date) result[@"taken_at"] = @((long long)date.timeIntervalSince1970);
                    }
                    if ([key containsString:@"location.ISO6709"]) {
                        const char *location = item.stringValue.UTF8String;
                        double lat, lng;
                        if (location && sscanf(location, "%lf%lf", &lat, &lng) == 2) { result[@"lat"] = @(lat); result[@"lng"] = @(lng); }
                    }
                }
            }
        } else {
            CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
            if (source) {
                NSDictionary *props = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
                if (props[(NSString *)kCGImagePropertyPixelWidth]) result[@"width"] = props[(NSString *)kCGImagePropertyPixelWidth];
                if (props[(NSString *)kCGImagePropertyPixelHeight]) result[@"height"] = props[(NSString *)kCGImagePropertyPixelHeight];
                int orientation=[props[(NSString *)kCGImagePropertyOrientation] intValue];
                if (orientation>=5 && orientation<=8) {id width=result[@"width"];result[@"width"]=result[@"height"];result[@"height"]=width;}
                id identifier = props[@"{MakerApple}"][@"17"];
                if ([identifier isKindOfClass:[NSString class]]) result[@"content_id"] = identifier;
                if (props[(NSString *)kCGImagePropertyProfileName]) result[@"color_profile"] = props[(NSString *)kCGImagePropertyProfileName];
                id original = props[(NSString *)kCGImagePropertyExifDictionary][(NSString *)kCGImagePropertyExifDateTimeOriginal];
                if ([original isKindOfClass:[NSString class]]) result[@"exif_datetime"] = original;
                if (@available(macOS 11.0, *)) {
                    CFDictionaryRef gain=CGImageSourceCopyAuxiliaryDataInfoAtIndex(source,0,kCGImageAuxiliaryDataTypeHDRGainMap);
                    result[@"hdr"]=[NSNumber numberWithBool:gain != NULL || [props[(NSString *)kCGImagePropertyDepth] intValue]>8];
                    if(gain)CFRelease(gain);
                }
                size_t count = CGImageSourceGetCount(source);
                result[@"frame_count"] = @(count);
                if (count > 1) {
                    double duration = 0;
                    for (size_t i = 0; i < count; i++) {
                        NSDictionary *frame = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, i, NULL));
                        NSDictionary *animation = frame[@"{GIF}"] ?: frame[@"{PNG}"] ?: frame[@"{WebP}"];
                        NSNumber *delay = animation[@"UnclampedDelayTime"] ?: animation[@"DelayTime"];
                        duration += delay ? MAX(0.01, delay.doubleValue) : 0.1;
                    }
                    result[@"anim_duration_ms"] = @((long long)(duration * 1000));
                }
                CFRelease(source);
            }
        }
        NSData *data = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String ?: "{}");
    }
}
void gphotos_free(char *value) { free(value); }
int gphotos_volume(const char *path, long long *available, long long *total) {
    @autoreleasepool {
        NSDictionary *values = [[NSURL fileURLWithPath:@(path)] resourceValuesForKeys:@[NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeTotalCapacityKey] error:NULL];
        NSNumber *avail = values[NSURLVolumeAvailableCapacityForImportantUsageKey], *size = values[NSURLVolumeTotalCapacityKey];
        if (!avail || !size) return 0;
        *available = avail.longLongValue; *total = size.longLongValue;
        return 1;
    }
}
static void drawAligned(CGContextRef context,CGImageRef image,CGImageRef reference,int width,int height) {
    VNTranslationalImageRegistrationRequest *request=[[VNTranslationalImageRegistrationRequest alloc] initWithTargetedCGImage:image options:@{}];
    VNImageRequestHandler *handler=[[VNImageRequestHandler alloc] initWithCGImage:reference options:@{}];
    CGAffineTransform transform=CGAffineTransformIdentity;
    if([handler performRequests:@[request] error:NULL] && request.results.count)transform=request.results.firstObject.alignmentTransform;
    double tx=MAX(-width*.06,MIN(width*.06,transform.tx*width/CGImageGetWidth(image)));
    double ty=MAX(-height*.06,MIN(height*.06,transform.ty*height/CGImageGetHeight(image)));
    CGContextSaveGState(context);
    CGContextTranslateCTM(context,-width*.075,-height*.075);
    CGContextScaleCTM(context,1.15,1.15);
    CGContextDrawImage(context,CGRectMake(tx,ty,width,height),image);
    CGContextRestoreGState(context);
}
static void loopRange(AVAssetImageGenerator *generator,double duration,double *start,double *end) {
    *start=0;*end=duration;
    if(duration<1.5 || duration>10)return;
    int count=MIN(120,(int)ceil(duration*30));
    NSMutableData *samples=[NSMutableData dataWithLength:count*4096];
    generator.maximumSize=CGSizeMake(64,64);
    CGColorSpaceRef gray=CGColorSpaceCreateDeviceGray();
    bool ok=true;
    for(int i=0;i<count;i++) {
        CGImageRef image=[generator copyCGImageAtTime:CMTimeMakeWithSeconds(duration*i/count,600) actualTime:NULL error:NULL];
        if(!image){ok=false;break;}
        CGContextRef context=CGBitmapContextCreate((unsigned char *)samples.mutableBytes+i*4096,64,64,8,64,gray,(CGBitmapInfo)kCGImageAlphaNone);
        if(!context){CGImageRelease(image);ok=false;break;}
        CGContextDrawImage(context,CGRectMake(0,0,64,64),image);CGContextRelease(context);CGImageRelease(image);
    }
    CGColorSpaceRelease(gray);
    if(!ok)return;
    unsigned char *pixels=samples.mutableBytes;double best=DBL_MAX;
    int separation=(int)ceil(count/duration);
    for(int i=0;i<count;i++)for(int j=i+separation;j<count;j++) {
        double score=0;
        for(int p=0;p<4096;p++) {double delta=pixels[i*4096+p]-pixels[j*4096+p];score+=delta*delta;
            if(i+1<count && j+1<count){double motion=(pixels[(i+1)*4096+p]-pixels[i*4096+p])-(pixels[(j+1)*4096+p]-pixels[j*4096+p]);score+=motion*motion*.25;}}
        if(score<best){best=score;*start=duration*i/count;*end=duration*j/count;}
    }
}
int gphotos_render(const char *path, const char *output, int bounce, unsigned edge, int animated) {
    @autoreleasepool {
        NSURL *url=[NSURL fileURLWithPath:@(path)];
        AVURLAsset *asset=[AVURLAsset URLAssetWithURL:url options:nil];
        AVAssetImageGenerator *generator=[AVAssetImageGenerator assetImageGeneratorWithAsset:asset];
        generator.appliesPreferredTrackTransform=YES; generator.maximumSize=CGSizeMake(edge,edge);
        generator.requestedTimeToleranceBefore=CMTimeMake(1,60);generator.requestedTimeToleranceAfter=CMTimeMake(1,60);
        CGImageSourceRef source=animated ? CGImageSourceCreateWithURL((__bridge CFURLRef)url,NULL) : NULL;
        NSMutableArray<NSNumber *> *ends=[NSMutableArray array];
        double duration=0;
        if (source) {
            for (size_t i=0;i<CGImageSourceGetCount(source);i++) {
                NSDictionary *p=CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source,i,NULL));
                NSDictionary *a=p[@"{GIF}"] ?: p[@"{PNG}"] ?: p[@"{WebP}"];
                NSNumber *delay=a[@"UnclampedDelayTime"] ?: a[@"DelayTime"];
                duration+=delay ? MAX(0.01,delay.doubleValue) : 0.1; [ends addObject:@(duration)];
            }
        } else duration=CMTimeGetSeconds(asset.duration);
        if (!isfinite(duration) || duration<=0 || duration>600) { if(source)CFRelease(source); return 0; }
        CGImageRef first=source ? CGImageSourceCreateImageAtIndex(source,0,NULL) : [generator copyCGImageAtTime:kCMTimeZero actualTime:NULL error:NULL];
        if (!first) {if(source)CFRelease(source);return 0;}
        double scale=MIN(1.,(double)edge/MAX(CGImageGetWidth(first),CGImageGetHeight(first)));
        int width=MAX(2,((int)(CGImageGetWidth(first)*scale)/2)*2),height=MAX(2,((int)(CGImageGetHeight(first)*scale)/2)*2);
        double start=0,end=duration;
        if(!source && !bounce)loopRange(generator,duration,&start,&end);
        generator.maximumSize=CGSizeMake(edge,edge);
        double crossfade=(!source && !bounce && end-start>.5) ? .25 : 0;
        duration=end-start-crossfade;
        AVAssetWriter *writer=[AVAssetWriter assetWriterWithURL:[NSURL fileURLWithPath:@(output)] fileType:AVFileTypeMPEG4 error:NULL];
        AVAssetWriterInput *input=[AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:@{AVVideoCodecKey:AVVideoCodecTypeH264,AVVideoWidthKey:@(width),AVVideoHeightKey:@(height)}];
        AVAssetWriterInputPixelBufferAdaptor *adaptor=[AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input sourcePixelBufferAttributes:@{(NSString *)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32ARGB),(NSString *)kCVPixelBufferWidthKey:@(width),(NSString *)kCVPixelBufferHeightKey:@(height),(NSString *)kCVPixelBufferCGImageCompatibilityKey:@YES,(NSString *)kCVPixelBufferCGBitmapContextCompatibilityKey:@YES}];
        if (!writer || ![writer canAddInput:input]) {CGImageRelease(first);if(source)CFRelease(source);return 0;}
        [writer addInput:input];
        if (![writer startWriting]) {CGImageRelease(first);if(source)CFRelease(source);return 0;}
        [writer startSessionAtSourceTime:kCMTimeZero];
        int count=MAX(1,(int)ceil(duration*30)),total=bounce==1 ? count*2 : count;
        bool ok=true;
        for (int i=0;i<total && ok;i++) { @autoreleasepool {
            int index=i<count ? i : total-1-i;
            double time=MIN(end-0.001,start+crossfade+index/30.);
            CGImageRef image=NULL;
            if (source) {
                NSUInteger frame=0;while(frame+1<ends.count && ends[frame].doubleValue<=time)frame++;
                image=CGImageSourceCreateThumbnailAtIndex(source,frame,(__bridge CFDictionaryRef)@{(NSString *)kCGImageSourceCreateThumbnailFromImageAlways:@YES,(NSString *)kCGImageSourceThumbnailMaxPixelSize:@(edge),(NSString *)kCGImageSourceCreateThumbnailWithTransform:@YES});
            } else image=[generator copyCGImageAtTime:CMTimeMakeWithSeconds(time,600) actualTime:NULL error:NULL];
            if (!image) {ok=false;break;}
            CVPixelBufferRef buffer=NULL;
            if (CVPixelBufferPoolCreatePixelBuffer(NULL,adaptor.pixelBufferPool,&buffer)!=kCVReturnSuccess) {CGImageRelease(image);ok=false;break;}
            CVPixelBufferLockBaseAddress(buffer,0);
            CGColorSpaceRef space=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            CGContextRef ctx=CGBitmapContextCreate(CVPixelBufferGetBaseAddress(buffer),width,height,8,CVPixelBufferGetBytesPerRow(buffer),space,(CGBitmapInfo)kCGImageAlphaNoneSkipFirst);
            CGColorSpaceRelease(space);
            if(ctx){
                if(source)CGContextDrawImage(ctx,CGRectMake(0,0,width,height),image);
                else {
                    drawAligned(ctx,image,first,width,height);
                    if(crossfade>0 && time>end-crossfade){
                        double blend=time-(end-crossfade);
                        CGImageRef overlay=[generator copyCGImageAtTime:CMTimeMakeWithSeconds(start+blend,600) actualTime:NULL error:NULL];
                        if(overlay){CGContextSetAlpha(ctx,blend/crossfade);drawAligned(ctx,overlay,first,width,height);CGImageRelease(overlay);}
                    }
                }
                CGContextRelease(ctx);
            }else ok=false;
            CGImageRelease(image);CVPixelBufferUnlockBaseAddress(buffer,0);
            NSDate *deadline=[NSDate dateWithTimeIntervalSinceNow:10];
            while(!input.readyForMoreMediaData && writer.status==AVAssetWriterStatusWriting && deadline.timeIntervalSinceNow>0)[NSThread sleepForTimeInterval:0.002];
            if(!input.readyForMoreMediaData || ![adaptor appendPixelBuffer:buffer withPresentationTime:CMTimeMake(i,30)])ok=false;
            CVPixelBufferRelease(buffer);
        }}
        CGImageRelease(first);
        if(source)CFRelease(source);
        if(!ok){[writer cancelWriting];return 0;}
        [input markAsFinished];
        dispatch_semaphore_t done=dispatch_semaphore_create(0);
        [writer finishWritingWithCompletionHandler:^{dispatch_semaphore_signal(done);}];
        if(dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,30*NSEC_PER_SEC))!=0){[writer cancelWriting];return 0;}
        return writer.status==AVAssetWriterStatusCompleted;
    }
}
int gphotos_animation_frame(const char *path,const char *output,long long ms,unsigned edge) {
    @autoreleasepool {
        CGImageSourceRef source=CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(path)],NULL);
        if(!source)return 0;
        double end=0;size_t chosen=0;
        for(size_t i=0;i<CGImageSourceGetCount(source);i++) {
            NSDictionary *p=CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source,i,NULL));
            NSDictionary *a=p[@"{GIF}"] ?: p[@"{PNG}"] ?: p[@"{WebP}"];
            NSNumber *delay=a[@"UnclampedDelayTime"] ?: a[@"DelayTime"];
            end+=(delay ? MAX(0.01,delay.doubleValue) : 0.1)*1000;chosen=i;
            if(end>ms)break;
        }
        CGImageRef image=CGImageSourceCreateThumbnailAtIndex(source,chosen,(__bridge CFDictionaryRef)@{(NSString *)kCGImageSourceCreateThumbnailFromImageAlways:@YES,(NSString *)kCGImageSourceThumbnailMaxPixelSize:@(edge ? edge : 32768),(NSString *)kCGImageSourceCreateThumbnailWithTransform:@YES});
        bool ok=writePreview(image,output);if(image)CGImageRelease(image);CFRelease(source);return ok;
    }
}
int gphotos_exposure(const char *path,const char *output,unsigned edge) {
    @autoreleasepool {
        AVURLAsset *asset=[AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:@(path)] options:nil];
        double duration=CMTimeGetSeconds(asset.duration);
        if(!isfinite(duration) || duration<=0 || duration>600)return 0;
        AVAssetImageGenerator *generator=[AVAssetImageGenerator assetImageGeneratorWithAsset:asset];
        generator.appliesPreferredTrackTransform=YES;generator.maximumSize=CGSizeMake(MIN(edge,2048),MIN(edge,2048));
        generator.requestedTimeToleranceBefore=CMTimeMake(1,60);generator.requestedTimeToleranceAfter=CMTimeMake(1,60);
        CGImageRef reference=[generator copyCGImageAtTime:kCMTimeZero actualTime:NULL error:NULL];
        if(!reference)return 0;
        size_t width=CGImageGetWidth(reference),height=CGImageGetHeight(reference),count=width*height*4;
        unsigned char *pixels=calloc(count,1);float *sum=calloc(count,sizeof(float));
        if(!pixels || !sum){free(pixels);free(sum);CGImageRelease(reference);return 0;}
        CGColorSpaceRef space=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context=CGBitmapContextCreate(pixels,width,height,8,width*4,space,(CGBitmapInfo)kCGImageAlphaNoneSkipLast);
        CGColorSpaceRelease(space);
        bool ok=context!=NULL;
        for(int i=0;i<30 && ok;i++) { @autoreleasepool {
            CGImageRef image=[generator copyCGImageAtTime:CMTimeMakeWithSeconds(duration*i/30.,600) actualTime:NULL error:NULL];
            if(!image){ok=false;break;}
            memset(pixels,0,count);drawAligned(context,image,reference,(int)width,(int)height);CGImageRelease(image);
            for(size_t p=0;p<count;p++)sum[p]+=powf(pixels[p]/255.f,2.2f)/30.f;
        }}
        if(ok){for(size_t p=0;p<count;p++)pixels[p]=(unsigned char)roundf(MIN(255.f,255.f*powf(sum[p],1.f/2.2f)));
            CGImageRef image=CGBitmapContextCreateImage(context);ok=writePreview(image,output);CGImageRelease(image);}
        if(context)CGContextRelease(context);free(pixels);free(sum);CGImageRelease(reference);return ok;
    }
}
char *gphotos_animation_samples(const char *path) {
    @autoreleasepool {
        CGImageSourceRef source=CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(path)],NULL);
        if(!source)return strdup("[]");
        size_t count=CGImageSourceGetCount(source);
        NSMutableArray<NSNumber *> *times=[NSMutableArray array];double ms=0;
        for(size_t i=0;i<count;i++) {
            [times addObject:@((long long)ms)];
            NSDictionary *p=CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source,i,NULL));
            NSDictionary *a=p[@"{GIF}"] ?: p[@"{PNG}"] ?: p[@"{WebP}"];
            NSNumber *delay=a[@"UnclampedDelayTime"] ?: a[@"DelayTime"];
            ms+=(delay ? MAX(.01,delay.doubleValue) : .1)*1000;
        }
        NSMutableIndexSet *selected=[NSMutableIndexSet indexSet];
        if(count<=8){[selected addIndexesInRange:NSMakeRange(0,count)];}
        else {
            for(int i=0;i<6;i++)[selected addIndex:(count-1)*i/5];
            CGColorSpaceRef gray=CGColorSpaceCreateDeviceGray();
            unsigned char previous[4096]={0},pixels[4096];
            NSMutableArray<NSDictionary *> *cuts=[NSMutableArray array];
            size_t candidates=MIN((size_t)24,count);
            for(size_t i=0;i<candidates;i++) {
                size_t index=(count-1)*i/(candidates-1);
                CGImageRef image=CGImageSourceCreateThumbnailAtIndex(source,index,(__bridge CFDictionaryRef)@{(NSString *)kCGImageSourceCreateThumbnailFromImageAlways:@YES,(NSString *)kCGImageSourceThumbnailMaxPixelSize:@64});
                if(!image)continue;
                CGContextRef context=CGBitmapContextCreate(pixels,64,64,8,64,gray,(CGBitmapInfo)kCGImageAlphaNone);
                if(!context){CGImageRelease(image);continue;}
                CGContextDrawImage(context,CGRectMake(0,0,64,64),image);CGContextRelease(context);CGImageRelease(image);
                if(i>0){double delta=0;for(int p=0;p<4096;p++)delta+=abs((int)pixels[p]-(int)previous[p]);delta/=4096.;if(delta>20 && ![selected containsIndex:index])[cuts addObject:@{@"index":@(index),@"score":@(delta)}];}
                memcpy(previous,pixels,4096);
            }
            CGColorSpaceRelease(gray);
            [cuts sortUsingComparator:^NSComparisonResult(NSDictionary *a,NSDictionary *b){return [b[@"score"] compare:a[@"score"]];}];
            for(NSDictionary *cut in cuts){if(selected.count>=8)break;[selected addIndex:[cut[@"index"] unsignedIntegerValue]];}
            for(size_t i=0;selected.count<8 && i<8;i++)[selected addIndex:(count-1)*i/7];
        }
        NSMutableArray *result=[NSMutableArray array];[selected enumerateIndexesUsingBlock:^(NSUInteger index,BOOL *stop){(void)stop;[result addObject:times[index]];}];
        CFRelease(source);
        NSData *data=[NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String ?: "[]");
    }
}
static void putValue(NSMutableDictionary *out, NSString *key, id value) {
    if ([value isKindOfClass:[NSString class]]) {
        NSString *text = [(NSString *)value stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (text.length) out[key] = text;
    } else if ([value isKindOfClass:[NSNumber class]]) {
        if (isfinite([(NSNumber *)value doubleValue])) out[key] = value;
    } else if ([value isKindOfClass:[NSArray class]]) {
        NSMutableArray *items = [NSMutableArray array];
        for (id item in (NSArray *)value) {
            if ([item isKindOfClass:[NSString class]] && [(NSString *)item length]) [items addObject:item];
            else if ([item isKindOfClass:[NSNumber class]] && isfinite([(NSNumber *)item doubleValue])) [items addObject:item];
        }
        if (items.count) out[key] = items;
    }
}
// Display-only details (camera, lens, exposure, codec…). Missing values are simply absent.
char *gphotos_details(const char *path, int video) {
    @autoreleasepool {
        NSMutableDictionary *out = [NSMutableDictionary dictionary];
        NSURL *url = [NSURL fileURLWithPath:@(path)];
        if (video) {
            AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
            NSMutableArray<AVMetadataItem *> *items = [NSMutableArray array];
            for (NSString *format in asset.availableMetadataFormats) [items addObjectsFromArray:[asset metadataForFormat:format]];
            for (AVMetadataItem *item in items) {
                NSString *key = [[item.key description] lowercaseString] ?: @"";
                NSString *common = item.commonKey ?: @"";
                id value = item.stringValue ?: item.numberValue;
                if (!value) continue;
                if ([common isEqualToString:AVMetadataCommonKeyMake] || [key hasSuffix:@"quicktime.make"]) putValue(out, @"make", value);
                else if ([common isEqualToString:AVMetadataCommonKeyModel] || [key hasSuffix:@"quicktime.model"]) putValue(out, @"model", value);
                else if ([common isEqualToString:AVMetadataCommonKeySoftware] || [key hasSuffix:@"quicktime.software"]) putValue(out, @"software", value);
                else if ([common isEqualToString:AVMetadataCommonKeyDescription]) putValue(out, @"description", value);
                else if ([common isEqualToString:AVMetadataCommonKeyAuthor] || [common isEqualToString:AVMetadataCommonKeyArtist]) putValue(out, @"artist", value);
                else if ([common isEqualToString:AVMetadataCommonKeyCopyrights]) putValue(out, @"copyright", value);
                else if ([key containsString:@"lens_model"]) putValue(out, @"lens_model", value);
                else if ([key containsString:@"focal_length.35mm_equivalent"]) putValue(out, @"focal_length_35mm", item.numberValue);
            }
            AVAssetTrack *track = [[asset tracksWithMediaType:AVMediaTypeVideo] firstObject];
            if (track) {
                if (track.nominalFrameRate > 0) out[@"frame_rate"] = @(track.nominalFrameRate);
                if (track.estimatedDataRate > 0) out[@"video_bitrate"] = @((long long)track.estimatedDataRate);
                id description = track.formatDescriptions.firstObject;
                if (description) {
                    FourCharCode code = CMFormatDescriptionGetMediaSubType((__bridge CMFormatDescriptionRef)description);
                    char text[5] = {(char)(code >> 24), (char)(code >> 16), (char)(code >> 8), (char)code, 0};
                    putValue(out, @"codec", @(text));
                    NSDictionary *extensions = (__bridge NSDictionary *)CMFormatDescriptionGetExtensions((__bridge CMFormatDescriptionRef)description);
                    id transfer = extensions[(NSString *)kCMFormatDescriptionExtension_TransferFunction];
                    if ([transfer isEqual:(NSString *)kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG] || [transfer isEqual:(NSString *)kCMFormatDescriptionTransferFunction_SMPTE_ST_2084_PQ]) out[@"hdr_video"] = @YES;
                }
            }
            AVAssetTrack *audio = [[asset tracksWithMediaType:AVMediaTypeAudio] firstObject];
            if (audio) {
                id description = audio.formatDescriptions.firstObject;
                const AudioStreamBasicDescription *basic = description ? CMAudioFormatDescriptionGetStreamBasicDescription((__bridge CMAudioFormatDescriptionRef)description) : NULL;
                if (basic) {
                    if (basic->mSampleRate > 0) out[@"audio_sample_rate"] = @(basic->mSampleRate);
                    if (basic->mChannelsPerFrame > 0) out[@"audio_channels"] = @(basic->mChannelsPerFrame);
                } else out[@"audio_channels"] = @0;
            }
        } else {
            CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
            if (source) {
                NSDictionary *props = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
                NSDictionary *tiff = props[(NSString *)kCGImagePropertyTIFFDictionary];
                NSDictionary *exif = props[(NSString *)kCGImagePropertyExifDictionary];
                NSDictionary *exifAux = props[(NSString *)kCGImagePropertyExifAuxDictionary];
                NSDictionary *gps = props[(NSString *)kCGImagePropertyGPSDictionary];
                NSDictionary *iptc = props[(NSString *)kCGImagePropertyIPTCDictionary];
                putValue(out, @"make", tiff[(NSString *)kCGImagePropertyTIFFMake]);
                putValue(out, @"model", tiff[(NSString *)kCGImagePropertyTIFFModel]);
                putValue(out, @"software", tiff[(NSString *)kCGImagePropertyTIFFSoftware]);
                putValue(out, @"artist", tiff[(NSString *)kCGImagePropertyTIFFArtist]);
                putValue(out, @"copyright", tiff[(NSString *)kCGImagePropertyTIFFCopyright]);
                putValue(out, @"description", tiff[(NSString *)kCGImagePropertyTIFFImageDescription]);
                putValue(out, @"lens_make", exif[(NSString *)kCGImagePropertyExifLensMake]);
                putValue(out, @"lens_model", exif[(NSString *)kCGImagePropertyExifLensModel] ?: exifAux[(NSString *)kCGImagePropertyExifAuxLensModel]);
                putValue(out, @"f_number", exif[(NSString *)kCGImagePropertyExifFNumber]);
                putValue(out, @"exposure_time", exif[(NSString *)kCGImagePropertyExifExposureTime]);
                id iso = exif[(NSString *)kCGImagePropertyExifISOSpeedRatings];
                putValue(out, @"iso", [iso isKindOfClass:[NSArray class]] ? [(NSArray *)iso firstObject] : iso);
                putValue(out, @"focal_length", exif[(NSString *)kCGImagePropertyExifFocalLength]);
                putValue(out, @"focal_length_35mm", exif[(NSString *)kCGImagePropertyExifFocalLenIn35mmFilm]);
                putValue(out, @"exposure_bias", exif[(NSString *)kCGImagePropertyExifExposureBiasValue]);
                putValue(out, @"flash", exif[(NSString *)kCGImagePropertyExifFlash]);
                putValue(out, @"white_balance", exif[(NSString *)kCGImagePropertyExifWhiteBalance]);
                putValue(out, @"metering_mode", exif[(NSString *)kCGImagePropertyExifMeteringMode]);
                putValue(out, @"exposure_program", exif[(NSString *)kCGImagePropertyExifExposureProgram]);
                putValue(out, @"scene_type", exif[(NSString *)kCGImagePropertyExifSceneCaptureType]);
                putValue(out, @"digital_zoom", exif[(NSString *)kCGImagePropertyExifDigitalZoomRatio]);
                putValue(out, @"subject_distance", exif[(NSString *)kCGImagePropertyExifSubjectDistance]);
                putValue(out, @"offset_time", exif[@"OffsetTimeOriginal"] ?: exif[@"OffsetTime"]);
                putValue(out, @"serial", exif[(NSString *)kCGImagePropertyExifBodySerialNumber] ?: exifAux[(NSString *)kCGImagePropertyExifAuxSerialNumber]);
                if ([gps[(NSString *)kCGImagePropertyGPSAltitude] isKindOfClass:[NSNumber class]]) {
                    double altitude = [gps[(NSString *)kCGImagePropertyGPSAltitude] doubleValue];
                    if ([gps[(NSString *)kCGImagePropertyGPSAltitudeRef] intValue] == 1) altitude = -altitude;
                    putValue(out, @"altitude", @(altitude));
                }
                putValue(out, @"direction", gps[(NSString *)kCGImagePropertyGPSImgDirection]);
                putValue(out, @"keywords", iptc[(NSString *)kCGImagePropertyIPTCKeywords]);
                if (!out[@"description"]) putValue(out, @"description", iptc[(NSString *)kCGImagePropertyIPTCCaptionAbstract]);
                if (!out[@"artist"]) putValue(out, @"artist", iptc[(NSString *)kCGImagePropertyIPTCByline]);
                putValue(out, @"bit_depth", props[(NSString *)kCGImagePropertyDepth]);
                putValue(out, @"color_model", props[(NSString *)kCGImagePropertyColorModel]);
                putValue(out, @"dpi", props[(NSString *)kCGImagePropertyDPIWidth]);
                CFRelease(source);
            }
        }
        NSData *data = [NSJSONSerialization dataWithJSONObject:out options:0 error:NULL];
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String ?: "{}");
    }
}
